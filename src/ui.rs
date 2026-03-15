use std::cell::{Ref, RefCell, RefMut};
use std::convert::*;
use std::path::*;
use std::rc::Rc;
use std::sync::Arc;
use std::{env, thread};

use log::{debug, warn};

use gio::prelude::*;
use gio::{ApplicationCommandLine, SimpleAction};
use gtk::{ApplicationWindow, Orientation, Paned, prelude::*};

use crate::Args;
use crate::highlight::BackgroundState;
use crate::misc::{self, BoolExt};
use crate::nvim::*;
use crate::settings::Settings;
use crate::shell::{self, Shell};
use crate::subscriptions::{SubscriptionHandle, SubscriptionKey};

const DEFAULT_WIDTH: i32 = 800;
const DEFAULT_HEIGHT: i32 = 600;
const DEFAULT_SIDEBAR_WIDTH: i32 = 200;

pub struct Ui {
    open_paths: Box<[String]>,
    initialized: bool,
    comps: Arc<UiMutex<Components>>,
    settings: Rc<RefCell<Settings>>,
    shell: Rc<RefCell<Shell>>,
}

pub struct Components {
    window: Option<ApplicationWindow>,
    window_state: ToplevelState,
    title_label: Option<gtk::Label>,
    pub exit_confirmed: bool,
}

impl Components {
    fn new() -> Components {
        Components {
            window: None,
            window_state: ToplevelState::default(),
            title_label: None,
            exit_confirmed: false,
        }
    }

    pub fn close_window(&self) {
        self.window.as_ref().unwrap().close();
    }

    pub fn window(&self) -> &ApplicationWindow {
        self.window.as_ref().unwrap()
    }

    pub fn set_title(&self, short_title: &str, long_title: &str) {
        self.window.as_ref().unwrap().set_title(Some(long_title));
        if let Some(ref title_label) = self.title_label {
            title_label.set_label(short_title);
        }
    }

    pub fn saved_size(&self) -> (i32, i32) {
        (
            self.window_state.current_width,
            self.window_state.current_height,
        )
    }
}

impl Ui {
    pub fn new(options: Args, open_paths: Box<[String]>) -> Ui {
        let comps = Arc::new(UiMutex::new(Components::new()));
        let settings = Rc::new(RefCell::new(Settings::new()));
        let shell = Rc::new(RefCell::new(Shell::new(settings.clone(), options)));
        settings.borrow_mut().set_shell(Rc::downgrade(&shell));

        #[allow(clippy::arc_with_non_send_sync)]
        Ui {
            initialized: false,
            comps,
            shell,
            settings,
            open_paths,
        }
    }

    pub fn init(
        &mut self,
        app: &gtk::Application,
        args: &crate::Args,
        app_cmdline: Rc<RefCell<Option<ApplicationCommandLine>>>,
    ) {
        if self.initialized {
            return;
        }
        self.initialized = true;

        let mut settings = self.settings.borrow_mut();
        settings.init();

        let window = ApplicationWindow::new(app);

        // For some reason, having a transparent window breaks window behavior on macOS.
        // See #46
        if cfg!(not(target_os = "macos")) {
            window.add_css_class("nvim-background");
        }

        let main = Paned::builder()
            .orientation(Orientation::Horizontal)
            .focusable(false)
            .build();

        let comps_ref = &self.comps;
        let shell_ref = &self.shell;

        {
            self.shell.borrow_mut().init(app_cmdline, comps_ref);

            // initialize window from comps
            // borrowing of comps must be leaved
            // for event processing
            let mut comps = comps_ref.borrow_mut();

            comps.window = Some(window.clone());

            let prefer_dark_theme = env::var("LEGUINVIM_PREFER_DARK_THEME")
                .map(|opt| opt.trim() == "1")
                .unwrap_or(false)
                || args.prefer_dark_theme;
            if prefer_dark_theme {
                window
                    .settings()
                    .set_property("gtk-application-prefer-dark-theme", true);
            }

            let sidebar_width = if !args.disable_win_restore {
                if comps.window_state.is_maximized {
                    window.maximize();
                }

                window.set_default_size(
                    comps.window_state.current_width,
                    comps.window_state.current_height,
                );

                comps.window_state.sidebar_width
            } else {
                window.set_default_size(DEFAULT_WIDTH, DEFAULT_HEIGHT);
                DEFAULT_SIDEBAR_WIDTH
            };

            main.set_position(if args.hide_sidebar { 0 } else { sidebar_width });
        }

        if args.no_window_decoration {
            window.set_decorated(false);
        }

        // Override default shortcuts which are easy to press accidentally
        if let Some(app) = window.application() {
            app.set_accels_for_action("app.preferences", &[]);
            app.set_accels_for_action("gtkinternal.hide", &[]);
            app.set_accels_for_action("gtkinternal.hide-others", &[]);
            app.set_accels_for_action("app.quit", &[]);
        }

        let show_sidebar_action =
            SimpleAction::new_stateful("show-sidebar", None, &false.to_variant());
        show_sidebar_action.connect_change_state(glib::clone!(
            #[weak]
            comps_ref,
            move |action, value| {
                if let Some(value) = value {
                    action.set_state(value);
                    let is_active = value.get::<bool>().unwrap();
                    comps_ref.borrow_mut().window_state.show_sidebar = is_active;
                }
            }
        ));
        app.add_action(&show_sidebar_action);

        window.connect_default_width_notify(glib::clone!(
            #[strong]
            main,
            #[weak]
            comps_ref,
            move |window| {
                gtk_window_resize(
                    window,
                    &mut comps_ref.borrow_mut(),
                    &main,
                    gtk::Orientation::Horizontal,
                );
            }
        ));
        window.connect_default_height_notify(glib::clone!(
            #[strong]
            main,
            #[weak]
            comps_ref,
            move |window| {
                gtk_window_resize(
                    window,
                    &mut comps_ref.borrow_mut(),
                    &main,
                    gtk::Orientation::Vertical,
                );
            }
        ));

        window.connect_maximized_notify(glib::clone!(
            #[weak]
            comps_ref,
            move |window| {
                comps_ref.borrow_mut().window_state.is_maximized = window.is_maximized();
            }
        ));

        let shell = self.shell.borrow();
        main.set_end_child(Some(&**shell));
        window.set_child(Some(&main));

        window.show();

        if !args.disable_win_restore {
            // Hide sidebar, if it wasn't shown last time.
            // Has to be done after show_all(), so it won't be shown again.
            let show_sidebar = comps_ref.borrow().window_state.show_sidebar;
            show_sidebar_action.change_state(&show_sidebar.to_variant());
        }

        let state_ref = shell_ref.borrow().state.clone();
        let state = state_ref.borrow();
        state.subscribe(
            SubscriptionKey::from("VimLeave"),
            &["v:exiting ? v:exiting : 0"],
            glib::clone!(
                #[weak]
                shell_ref,
                move |args| set_exit_code(&shell_ref, args)
            ),
        );

        // Autocmds we want to run when starting
        let autocmds = vec![
            state.subscribe(
                SubscriptionKey::from("BufEnter,BufFilePost,BufModifiedSet,DirChanged"),
                &[
                    "expand('%:p')",
                    "getcwd()",
                    "argidx()",
                    "argc()",
                    "&modified",
                    "&modifiable",
                    "win_gettype()",
                    "&buftype",
                ],
                glib::clone!(
                    #[weak]
                    comps_ref,
                    move |args| update_window_title(&comps_ref, args)
                ),
            ),
            state.subscribe(
                SubscriptionKey::with_pattern("OptionSet", "background"),
                &["&background"],
                glib::clone!(
                    #[weak]
                    shell_ref,
                    move |args| set_background(&shell_ref, args)
                ),
            ),
        ];

        shell.grab_focus();

        let (post_config_cmds, diff_mode) = {
            let mut options = state.options.borrow_mut();

            (options.post_config_cmds(), options.diff_mode)
        };

        drop(state);
        shell.set_detach_cb(Some(glib::clone!(
            #[strong]
            comps_ref,
            move || {
                glib::idle_add_once(glib::clone!(
                    #[strong]
                    comps_ref,
                    move || comps_ref.borrow().close_window()
                ));
            }
        )));

        shell.set_nvim_started_cb(Some(glib::clone!(
            #[strong(rename_to = files_list)]
            self.open_paths,
            move || {
                Ui::nvim_started(
                    &state_ref.borrow(),
                    &files_list,
                    &autocmds,
                    post_config_cmds.as_ref(),
                    diff_mode,
                );
            }
        )));

        let sidebar_action = UiMutex::new(show_sidebar_action);
        let comps_ref = comps_ref.clone();
        shell.set_nvim_command_cb(Some(
            move |shell: &mut shell::State, command: NvimCommand| {
                Ui::nvim_command(shell, command, &sidebar_action, &comps_ref);
            },
        ));
    }

    fn nvim_started(
        shell: &shell::State,
        files_list: &[String],
        subscriptions: &[SubscriptionHandle],
        post_config_cmds: &[String],
        diff_mode: bool,
    ) {
        shell.set_autocmds();
        for subscription in subscriptions.iter() {
            shell.run_now(subscription);
        }

        let mut commands = Vec::<String>::new();
        if !files_list.is_empty() {
            if diff_mode {
                commands.reserve(files_list.len() + post_config_cmds.len());
                commands.push(format!(
                    r"try|e {}|cat /^Vim(\a\+):E325:/|endt|difft",
                    misc::escape_filename(&files_list[0])
                ));
                for file in &files_list[1..] {
                    commands.push(format!(
                        r"try|vs {}|cat /^Vim(\a\+):E325:/|endt|difft",
                        misc::escape_filename(file)
                    ));
                }
            } else {
                commands.reserve(1 + post_config_cmds.len());
                commands.push(format!(
                    r"try|ar {}|cat /^Vim(\a\+):E325:/|endt",
                    files_list
                        .iter()
                        .map(|f| misc::escape_filename(f))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        }

        commands.extend(
            post_config_cmds
                .iter()
                .map(|cmd| format!(r#"exec "{}""#, misc::viml_escape(cmd))),
        );
        debug!("{commands:?}");

        // open files as last command
        // because it can generate user query
        let action_widgets = shell.action_widgets();
        if commands.is_empty() {
            if let Some(action_widgets) = action_widgets.borrow().as_ref() {
                action_widgets.set_enabled(true);
            }
            return;
        }

        let commands = commands.join("|");
        let nvim_client = shell.nvim_clone();
        let nvim = nvim_client.nvim().unwrap();
        let channel_id = nvim_client
            .api_info()
            .expect("API info should be initialized by the time this is called")
            .channel;
        nvim.clone().spawn(async move {
            let res = nvim.command(&commands).await;

            glib::idle_add_once(move || {
                if let Some(action_widgets) = action_widgets.borrow().as_ref() {
                    action_widgets.set_enabled(true);
                }
            });

            if let Err(e) = res {
                if let Ok(e) = NormalError::try_from(&*e) {
                    if e == NormalError::KeyboardInterrupt {
                        nvim.shutdown(channel_id).await;
                    } else if !e.has_code(325) {
                        // Filter out errors we get if the user is presented with a prompt
                        e.print(&nvim).await;
                    }
                } else {
                    e.print();
                }
            }
        });
    }

    fn nvim_command(
        shell: &mut shell::State,
        command: NvimCommand,
        sidebar_action: &UiMutex<SimpleAction>,
        comps: &UiMutex<Components>,
    ) {
        match command {
            NvimCommand::ShowProjectView => {}
            NvimCommand::ShowGtkInspector => {
                comps
                    .borrow()
                    .window
                    .as_ref()
                    .unwrap()
                    .emit_enable_debugging(false);
            }
            NvimCommand::ToggleSidebar => {
                let action = sidebar_action.borrow();
                let state = !bool::from_variant(&action.state().unwrap()).unwrap();
                action.change_state(&state.to_variant());
            }
            NvimCommand::Transparency(background_alpha, filled_alpha) => {
                let comps = comps.borrow();
                let window = comps.window.as_ref().unwrap();

                let display = gtk::prelude::WidgetExt::display(window);
                if display.is_composited() {
                    shell.set_transparency(background_alpha, filled_alpha);
                } else {
                    warn!("Screen is not composited");
                }
            }
            NvimCommand::PreferDarkTheme(prefer_dark_theme) => {
                let comps = comps.borrow();
                let window = comps.window.as_ref().unwrap();

                window
                    .settings()
                    .set_property("gtk-application-prefer-dark-theme", prefer_dark_theme);
            }
        }
    }
}

fn gtk_window_resize(
    app_window: &gtk::ApplicationWindow,
    comps: &mut Components,
    main: &Paned,
    orientation: gtk::Orientation,
) {
    if !app_window.is_maximized() {
        match orientation {
            gtk::Orientation::Horizontal => {
                comps.window_state.current_width = app_window.size(gtk::Orientation::Horizontal)
            }
            gtk::Orientation::Vertical => {
                comps.window_state.current_height = app_window.size(gtk::Orientation::Vertical)
            }
            _ => unreachable!(),
        }
    }
    if comps.window_state.show_sidebar {
        comps.window_state.sidebar_width = main.position();
    }
}

fn set_background(shell: &RefCell<Shell>, args: Vec<String>) {
    let background = match args[0].as_str() {
        "light" => BackgroundState::Light,
        "dark" => BackgroundState::Dark,
        val => panic!("Unexpected 'background' value received: {}", val),
    };

    let state = &shell.borrow().state;
    state.borrow().set_background(background);

    // Neovim won't send us a redraw to update the default colors on the screen, so do it ourselves
    glib::idle_add_once(glib::clone!(
        #[strong]
        state,
        move || state.borrow_mut().queue_draw(RedrawMode::ClearCache)
    ));
}

fn shorten_home_dir(path: impl AsRef<Path>) -> Option<String> {
    let path = path.as_ref();
    if let Ok(path) = path.canonicalize()
        && let Ok(path) = path.strip_prefix(glib::home_dir())
    {
        return Some(format!("~{MAIN_SEPARATOR}{}", path.to_string_lossy()));
    }

    None
}

fn format_window_title(
    file_path: &str,
    dir: &Path,
    argidx: u32,
    argc: u32,
    modified: bool,
    modifiable: bool,
    long: bool,
) -> String {
    let mut parts = Vec::with_capacity(5);

    let file_str;
    parts.push(if file_path.is_empty() {
        "[No Name]"
    } else if let Some(rel_path) = Path::new(file_path)
        .strip_prefix(dir)
        .ok()
        .and_then(|p| p.to_str())
    {
        rel_path
    } else if let Some(short_path) = shorten_home_dir(file_path) {
        file_str = short_path;
        &file_str
    } else {
        file_path
    });

    if modifiable {
        if modified {
            parts.push("+");
        }
    } else {
        parts.push("-");
    }

    let dir_str;
    if long {
        dir_str = format!(
            "({})",
            shorten_home_dir(dir).unwrap_or_else(|| dir.to_string_lossy().to_string())
        );
        parts.push(&dir_str);
    }

    let arg_cnt;
    if argc > 1 {
        arg_cnt = format!("({argidx} of {argc})");
        parts.push(&arg_cnt);
    }

    if long {
        parts.push("- leguinvim");
    }

    parts.join(" ")
}

fn update_window_title(comps: &Arc<UiMutex<Components>>, args: Vec<String>) {
    let file_path = &args[0];
    let dir = Path::new(&args[1]);
    let argidx = args[2].parse::<u32>().unwrap() + 1;
    let argc = args[3].parse::<u32>().unwrap();
    let modified = bool::from_int_str(&args[4]).unwrap();
    let modifiable = bool::from_int_str(&args[5]).unwrap();

    // Ignore certain window types that will never have a title (GH #26)
    let win_type = &args[6];
    let buf_type = &args[7];
    if !win_type.is_empty() || !matches!(buf_type.as_str(), "" | "terminal") {
        return;
    }

    comps.borrow().set_title(
        &format_window_title(file_path, dir, argidx, argc, modified, modifiable, false),
        &format_window_title(file_path, dir, argidx, argc, modified, modifiable, true),
    );
}

fn set_exit_code(shell: &RefCell<Shell>, args: Vec<String>) {
    let code = args[0].parse::<u8>().unwrap();
    shell.borrow().set_exit_code(code.into());
}

struct ToplevelState {
    current_width: i32,
    current_height: i32,
    is_maximized: bool,
    show_sidebar: bool,
    sidebar_width: i32,
}

impl Default for ToplevelState {
    fn default() -> Self {
        ToplevelState {
            current_width: DEFAULT_WIDTH,
            current_height: DEFAULT_HEIGHT,
            is_maximized: false,
            show_sidebar: false,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        }
    }
}

/// Our big thread-safety guard. This guard relies on the following assertions to remain true in
/// order to provide safety:
///
/// 1. T may never be accessed, except from within the same thread the UiMutex was originally
///    created on
/// 2. The thread T was created on is destroyed only after all other possible threads with
///    references to T have been finished execution
///
/// Both of these assumptions are verified at runtime, just in case.
#[derive(Debug)]
pub struct UiMutex<T: ?Sized> {
    thread: thread::ThreadId,
    data: RefCell<T>,
}

unsafe impl<T: ?Sized> Send for UiMutex<T> {}
unsafe impl<T: ?Sized> Sync for UiMutex<T> {}

impl<T: ?Sized> Drop for UiMutex<T> {
    fn drop(&mut self) {
        assert_eq!(
            self.thread,
            thread::current().id(),
            "Value dropped on a different thread than where it was created, this likely means our \
            async runtime outlived GTK+. That's not good!"
        );
    }
}

impl<T> UiMutex<T> {
    pub fn new(t: T) -> UiMutex<T> {
        UiMutex {
            thread: thread::current().id(),
            data: RefCell::new(t),
        }
    }

    pub fn replace(&self, t: T) -> T {
        self.assert_ui_thread();
        self.data.replace(t)
    }
}

impl<T: ?Sized> UiMutex<T> {
    pub fn borrow(&self) -> Ref<'_, T> {
        self.assert_ui_thread();
        self.data.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        self.assert_ui_thread();
        self.data.borrow_mut()
    }

    #[inline]
    fn assert_ui_thread(&self) {
        if thread::current().id() != self.thread {
            panic!("Can access to UI only from main thread");
        }
    }
}
