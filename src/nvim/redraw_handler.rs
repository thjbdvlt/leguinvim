use std::{convert::*, num::ParseFloatError, result, sync::Arc};

use nvim_rs::Value;

use log::{debug, error, warn};

use crate::nvim::{NvimSession, Tabpage};
use crate::shell;
use crate::ui::UiMutex;

use crate::value::ValueMapExt;

/// Indicates whether we should queue a draw and if so, whether we should invalidate any internal
/// caches before doing so
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RedrawMode {
    /// No redraw required
    Nothing,
    /// A redraw is required, but only the state of the cursor has changed
    Cursor,
    /// A redraw is required and no glyphs have changed, but the snapshot cache must be cleared
    /// anyway
    ClearCache,
    /// A redraw is required and glyphs have changed
    All,
}

macro_rules! try_str {
    ($exp:expr) => {
        $exp.as_str()
            .ok_or_else(|| "Can't convert argument to string".to_owned())?
    };
}

macro_rules! try_string {
    ($exp:expr) => {
        if let Value::String(s) = $exp {
            if let Some(s) = s.into_str() {
                Ok(s)
            } else {
                Err("Can't convert to utf8 string".to_owned())
            }
        } else {
            Err("Can't convert to string".to_owned())
        }
    };
}

macro_rules! try_int {
    ($expr:expr) => {
        $expr
            .as_i64()
            .ok_or_else(|| "Can't convert argument to int".to_owned())?
    };
}

macro_rules! try_uint {
    ($exp:expr) => {
        $exp.as_u64()
            .ok_or_else(|| "Can't convert argument to u64".to_owned())?
    };
}

macro_rules! try_float {
    ($exp:expr) => {
        $exp.as_f64()
            .ok_or_else(|| "Can't convert argument to f64".to_owned())?
    };
}

// Neovim will often represent optional uint values as a -1 to represent None
macro_rules! try_option_uint {
    ($exp:expr) => {{
        let val = $exp;
        if let Some(uint) = val.as_u64() {
            Ok(Some(uint))
        } else if val.as_i64() == Some(-1) {
            Ok(None)
        } else {
            Err(format!("Can't convert argument {} to Option<u64>", val))
        }?
    }};
}

// Note that Neovim doesn't actually have a u32 type. Or at the very least, there is not one
// documented in :help api-types . We mainly use these for places where we technically expect to
// receive a u64 or -1, but are not able to support anything larger then u32 due to some GTK+ api
// requiring a u32. At least as of writing this comment, this is mainly just for the external popup
// menu.
//
// This is definitely not an ideal solution, however: we still have a range of 0-4,294,967,295 which
// we should hopefully almost never go over in the real world.
// macro_rules! try_option_u32 {
//     ($exp:expr) => {
//         if let Some(uint) = try_option_uint!($exp) {
//             Some(u32::try_from(uint).map_err(|e| e.to_string())?)
//         } else {
//             None
//         }
//     };
// }

macro_rules! try_bool {
    ($exp:expr) => {
        $exp.as_bool()
            .ok_or_else(|| "Can't convert argument to bool".to_owned())?
    };
}

macro_rules! map_array {
    ($arg:expr, $err:expr, | $item:ident | $exp:expr) => {
        $arg.as_array().ok_or_else(|| $err).and_then(|items| {
            items
                .iter()
                .map(|$item| $exp)
                .collect::<Result<Vec<_>, _>>()
        })
    };
    ($arg:expr, $err:expr, | $item:ident |  { $exp:expr }) => {
        $arg.as_array().ok_or_else(|| $err).and_then(|items| {
            items
                .iter()
                .map(|$item| $exp)
                .collect::<Result<Vec<_>, _>>()
        })
    };
}

// macro_rules! into_array {
//     ($arg:expr, $err:expr, | $item:ident | $exp:expr) => {
//         if let Value::Array(items) = $arg {
//             items
//                 .into_iter()
//                 .map(|$item| $exp)
//                 .collect::<Result<Vec<_>, _>>()
//         } else {
//             Err($err.to_owned())
//         }
//     };
// }

macro_rules! try_arg {
    ($value:expr,val_ref) => {
        &$value
    };
    ($value:expr,val) => {
        $value
    };
    ($value:expr,bool) => {
        try_bool!($value)
    };
    ($value:expr,uint) => {
        try_uint!($value)
    };
    ($value:expr,opt_uint) => {
        try_option_uint!($value)
    };
    ($value:expr,int) => {
        try_int!($value)
    };
    ($value:expr,float) => {
        try_float!($value)
    };
    ($value:expr,str) => {
        match $value {
            Value::String(s) => {
                if let Some(s) = s.into_str() {
                    Ok(s)
                } else {
                    Err("Can't convert to utf8 string".to_owned())
                }
            }
            _ => Err("Can't convert to string".to_owned()),
        }?
    };
    ($value:expr,ext) => {
        rmpv::ext::from_value($value).map_err(|e| e.to_string())?
    };
}

macro_rules! call {
    ($s:ident -> $c:ident ($args:ident : $($arg_type:ident),+ )) => (
        {
            let mut iter = $args.into_iter();
            $s.$c($(
                try_arg!(iter.next()
                             .ok_or_else(|| format!("No such argument for {}", stringify!($c)))?,
                         $arg_type
                        )
            ),+ )
        }
    )
}

fn set_ui_opt(nvim: &NvimSession, opts: &[(&str, bool)], enable: bool) -> Result<(), String> {
    for (ext, supported) in opts {
        if !supported {
            return if enable {
                Err(format!(
                    "{ext} is not supported by your version of neovim, please update!"
                ))
            } else {
                Ok(())
            };
        }
    }

    for (ext, _) in opts {
        nvim.block_timeout(nvim.ui_set_option(ext, enable.into()))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub enum NvimCommand {
    ToggleSidebar,
    ShowProjectView,
    ShowGtkInspector,
    Transparency(f64, f64),
    PreferDarkTheme(bool),
}

pub fn call_gui_event(
    ui: &mut shell::State,
    method: &str,
    args: Vec<Value>,
) -> result::Result<(), String> {
    match method {
        "FontMono" => call!(ui->set_font_mono(args: str)),
        "Font" => call!(ui->set_font(args: str)),
        "MonoFloat" => call!(ui->set_mono_float(args: uint)),
        "FontFeatures" => call!(ui->set_font_features(args: str)),
        "Linespace" => call!(ui->set_line_space(args: str)),
        "Clipboard" => match try_str!(args[0]) {
            "Set" => match try_str!(args[1]) {
                "*" => ui.clipboard_primary_set(try_str!(args[2])),
                _ => ui.clipboard_clipboard_set(try_str!(args[2])),
            },
            opt => error!("Unknown option {opt}"),
        },
        "Option" => {
            let nvim_client = ui.nvim_clone();
            let nvim = nvim_client.nvim().ok_or("No nvim client")?;
            let api_info = nvim_client.api_info();

            let mut args = args.into_iter();
            let opt_name = try_string!(args.next().ok_or("UI extension name missing")?)?;
            let opt_value =
                try_int!(args.next().ok_or("New value for UI extension is missing")?) != 0;
            match opt_name.as_str() {
                "Tabline" => {
                    set_ui_opt(
                        &nvim,
                        &[(
                            "ext_tabline",
                            api_info
                                .as_ref()
                                .map(|api_info| api_info.ext_tabline)
                                .unwrap_or_default(),
                        )],
                        opt_value,
                    )?;
                    ui.set_tabline(opt_value);
                }
                opt => error!("Unknown option {opt}"),
            }
        }
        "Command" => {
            match try_str!(args[0]) {
                "ToggleSidebar" => ui.on_command(NvimCommand::ToggleSidebar),
                "ShowProjectView" => ui.on_command(NvimCommand::ShowProjectView),
                "ShowGtkInspector" => ui.on_command(NvimCommand::ShowGtkInspector),
                "Transparency" => ui.on_command(NvimCommand::Transparency(
                    try_str!(args.get(1).cloned().unwrap_or_else(|| "1.0".into()))
                        .parse()
                        .map_err(|e: ParseFloatError| e.to_string())?,
                    try_str!(args.get(2).cloned().unwrap_or_else(|| "1.0".into()))
                        .parse()
                        .map_err(|e: ParseFloatError| e.to_string())?,
                )),
                "PreferDarkTheme" => {
                    let prefer_dark_theme = matches!(
                        try_str!(args.get(1).cloned().unwrap_or_else(|| Value::from("off"))),
                        "on"
                    );

                    ui.on_command(NvimCommand::PreferDarkTheme(prefer_dark_theme))
                }
                "SetCursorBlink" => {
                    let blink_count =
                        try_str!(args.get(1).cloned().unwrap_or_else(|| Value::from(-1)))
                            .parse::<i32>()
                            .unwrap_or(-1);
                    ui.set_cursor_blink(blink_count);
                }
                _ => error!("Unknown command"),
            };
        }
        _ => return Err(format!("Unsupported event {method}({args:?})")),
    }
    Ok(())
}

pub fn call_gui_request(
    ui: &Arc<UiMutex<shell::State>>,
    method: &str,
    args: &Vec<Value>,
) -> result::Result<Value, Value> {
    match method {
        "Clipboard" => {
            match try_str!(args[0]) {
                "Get" => {
                    // NOTE: wait_for_text waits on the main loop. We can't have the ui borrowed
                    // while it runs, otherwise ui callbacks will get called and try to borrow
                    // mutably twice!
                    let clipboard = {
                        let ui = &ui.borrow();
                        match try_str!(args[1]) {
                            "*" => ui.clipboard_primary.clone(),
                            _ => ui.clipboard_clipboard.clone(),
                        }
                    };
                    let t = glib::MainContext::default()
                        .block_on(clipboard.read_text_future())
                        .unwrap_or(None)
                        .unwrap_or_else(|| "".into())
                        .to_string()
                        .replace("\r", "\n");

                    //Ok(Value::Array(vec![t.split('\n').to_owned()]))

                    /* Hello darkness my old friend
                     * no.
                     * I don't know why on earth we have suddenly gotten '\r' instead of '\n' in
                     * pastes. But right now I've got more important stuff going in my life.
                     * it literally happened one day
                     * if you know, please contact lyude for a $0 reward
                     */
                    Ok(Value::Array(
                        t.replace('\r', "\n")
                            .split('\n')
                            .map(|s| s.into())
                            .collect::<Vec<Value>>(),
                    ))
                }
                opt => Err(format!("Unknown option: {opt}").into()),
            }
        }
        _ => Err(format!("Unsupported request {method}({args:?})").into()),
    }
}

pub fn call(
    ui: &mut shell::State,
    method: &str,
    args: Vec<Value>,
) -> result::Result<RedrawMode, String> {
    let mut flush = false;
    let repaint_mode = match method {
        "win_pos" => {
            call!(ui->win_pos(args: uint, val_ref, int, int, uint, uint))
        }
        "win_close" => {
            call!(ui->win_close(args: uint))
        }
        "win_hide" => {
            call!(ui->win_hide(args: uint))
        }
        "win_float_pos" => {
            call!(ui->win_float_pos(args: uint, val_ref, str, uint, float, float, bool, uint))
        }
        "msg_set_pos" => {
            call!(ui->msg_set_pos(args: uint, uint, bool, str))
        }
        "win_external_pos" => RedrawMode::Nothing,

        // We ignore win_viewport, as we don't implement smooth scrolling.
        "win_viewport" => RedrawMode::Nothing,
        "win_viewport_margins" => RedrawMode::Nothing,

        "grid_line" => {
            call!(ui->grid_line(args: uint, uint, uint, ext))
        }
        "grid_clear" => {
            call!(ui->grid_clear(args: uint))
        }
        "grid_destroy" => call!(ui->grid_destroy(args: uint)),
        "grid_cursor_goto" => call!(ui->grid_cursor_goto(args: uint, uint, uint)),
        "grid_scroll" => call!(ui->grid_scroll(args: uint, uint, uint, uint, uint, int, int)),
        "grid_resize" => {
            call!(ui->grid_resize(args: uint, uint, uint))
        }

        "default_colors_set" => {
            call!(ui->default_colors_set(args: opt_uint, opt_uint, opt_uint, int, int))
        }
        "hl_attr_define" => call!(ui->hl_attr_define(args: uint, ext, val_ref, ext)),
        "mode_change" => call!(ui->on_mode_change(args: str, uint)),
        "mouse_on" => ui.on_mouse(true),
        "mouse_off" => ui.on_mouse(false),
        "busy_start" => ui.on_busy(true),
        "busy_stop" => ui.on_busy(false),
        "popupmenu_show" => {
            call!(ui->popupmenu_show(args: val_ref, int, int, int, int))
        }
        "popupmenu_select" => {
            call!(ui->popupmenu_select(args: int))
        }
        "popupmenu_hide" => ui.popupmenu_hide(),
        "tabline_update" => {
            if let Some(nvim) = ui.nvim() {
                let tabs_out = map_array!(args[1], "Error get tabline list".to_owned(), |tab| tab
                    .as_map()
                    .ok_or_else(|| "Error get map for tab".to_owned())
                    .and_then(|tab_map| tab_map.to_attrs_map())
                    .map(|tab_attrs| {
                        let name_attr = tab_attrs
                            .get("name")
                            .and_then(|n| n.as_str().map(|s| s.to_owned()));
                        let tab_attr = tab_attrs
                            .get("tab")
                            .map(|&tab_id| Tabpage::new(tab_id.clone(), (*nvim).clone()))
                            .unwrap();

                        (tab_attr, name_attr)
                    }))?;
                ui.tabline_update(Tabpage::new(args[0].clone(), (*nvim).clone()), tabs_out)
            } else {
                RedrawMode::Nothing
            }
        }
        "mode_info_set" => call!(ui->mode_info_set(args: bool, ext)),
        "option_set" => call!(ui->option_set(args: str, val)),

        "hl_group_set" => RedrawMode::Nothing, // TODO?
        "set_title" => RedrawMode::Nothing,    // TODO?
        "chdir" => RedrawMode::Nothing,        // TODO?
        "set_icon" => RedrawMode::Nothing,     // We ignore this.
        "update_menu" => RedrawMode::Nothing,  // We ignore this?

        "flush" => {
            debug!("Flush ({:?})", ui.pending_redraw);
            flush = true;
            ui.pending_redraw
        }
        _ => {
            warn!("Event {method}({args:?})");
            RedrawMode::Nothing
        }
    };

    if flush {
        ui.pending_redraw = RedrawMode::Nothing;
        Ok(repaint_mode)
    } else {
        ui.pending_redraw = ui.pending_redraw.max(repaint_mode);
        Ok(RedrawMode::Nothing)
    }
}
