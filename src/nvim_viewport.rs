use once_cell::sync::Lazy;

use gtk::{graphene::Rect, prelude::*, subclass::prelude::*};

use std::{
    cell::RefCell,
    sync::{Arc, Weak},
};

use crate::{
    render::*,
    shell::{RenderState, State},
    ui::UiMutex,
};

glib::wrapper! {
    pub struct NvimViewport(ObjectSubclass<NvimViewportObject>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl NvimViewport {
    pub fn new() -> Self {
        glib::Object::new::<Self>()
    }

    pub fn set_shell_state(&self, state: &Arc<UiMutex<State>>) {
        self.set_property("shell-state", glib::BoxedAnyObject::new(state.clone()));
    }

    pub fn set_context_menu(&self, popover_menu: &gtk::PopoverMenu) {
        self.set_property("context-menu", popover_menu);
    }

    pub fn clear_snapshot_cache(&self) {
        self.set_property("snapshot-cached", false);
    }
}

/** The inner state structure for the viewport widget, for holding non-glib types (e.g. ones that
 * need inferior mutability) */
#[derive(Default)]
struct NvimViewportInner {
    state: Weak<UiMutex<State>>,
    snapshot_cache: Option<gsk::RenderNode>,
}

#[derive(Default)]
pub struct NvimViewportObject {
    inner: RefCell<NvimViewportInner>,
    context_menu: glib::WeakRef<gtk::PopoverMenu>,
}

#[glib::object_subclass]
impl ObjectSubclass for NvimViewportObject {
    const NAME: &'static str = "NvimViewport";
    type Type = NvimViewport;
    type ParentType = gtk::Widget;

    fn class_init(klass: &mut Self::Class) {
        klass.set_css_name("widget");
        klass.set_accessible_role(gtk::AccessibleRole::TextBox);
    }
}

impl ObjectImpl for NvimViewportObject {
    fn dispose(&self) {
        if let Some(popover_menu) = self.context_menu.upgrade() {
            popover_menu.unparent();
        }
    }

    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
            vec![
                glib::ParamSpecObject::builder::<glib::BoxedAnyObject>("shell-state")
                    .write_only()
                    .build(),
                glib::ParamSpecBoolean::builder("snapshot-cached").build(),
                glib::ParamSpecObject::builder::<gtk::PopoverMenu>("context-menu").build(),
                glib::ParamSpecObject::builder::<gtk::Popover>("ext-cmdline").build(),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        let obj = self.obj();
        match pspec.name() {
            "shell-state" => {
                let mut inner = self.inner.borrow_mut();
                debug_assert!(inner.state.upgrade().is_none());

                inner.state =
                    Arc::downgrade(&value.get::<glib::BoxedAnyObject>().unwrap().borrow());
            }
            "snapshot-cached" => {
                if !value.get::<bool>().unwrap() {
                    self.inner.borrow_mut().snapshot_cache = None;
                }
            }
            "context-menu" => {
                if let Some(context_menu) = self.context_menu.upgrade() {
                    context_menu.unparent();
                }
                let context_menu: gtk::PopoverMenu = value.get().unwrap();

                context_menu.set_parent(&*obj);
                self.context_menu.set(Some(&context_menu));
            }
            _ => unreachable!(),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "snapshot-cached" => self.inner.borrow().snapshot_cache.is_some().to_value(),
            "context-menu" => self.context_menu.upgrade().to_value(),
            _ => unreachable!(),
        }
    }
}

impl WidgetImpl for NvimViewportObject {
    fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
        self.parent_size_allocate(width, height, baseline);
        if let Some(context_menu) = self.context_menu.upgrade() {
            context_menu.present();
        }
        let inner = self.inner.borrow();
        if let Some(state) = inner.state.upgrade() {
            state.borrow_mut().try_nvim_resize();
        }
    }

    fn snapshot(&self, snapshot_in: &gtk::Snapshot) {
        let obj = self.obj();
        let mut inner = self.inner.borrow_mut();
        let state = match inner.state.upgrade() {
            Some(state) => state,
            None => return,
        };
        let state = state.borrow();
        let render_state = state.render_state.borrow();
        let hl = &render_state.hl;

        // Draw the background first, to help GTK+ better notice that this doesn't change often
        let transparency = state.transparency();
        snapshot_in.append_color(
            &hl.bg().to_rgbo(transparency.background_alpha),
            &Rect::new(0.0, 0.0, obj.width() as f32, obj.height() as f32),
        );

        if state.nvim_clone().is_initialized() {
            // Render scenes get pretty huge here, so we cache them as often as possible
            let font_ctx = &render_state.font_ctx;
            let cell_metrics = font_ctx.cell_metrics();
            let mono_ctx = &render_state.mono_ctx;
            let mono_metrics = mono_ctx.cell_metrics();
            if inner.snapshot_cache.is_none() {
                inner.snapshot_cache =
                    snapshot_all_grids(cell_metrics, mono_metrics, &state.grids, hl);
            }
            if let Some(ref cached_snapshot) = inner.snapshot_cache {
                let push_opacity = transparency.filled_alpha < 0.99999;
                if push_opacity {
                    snapshot_in.push_opacity(transparency.filled_alpha)
                }

                snapshot_in.append_node(cached_snapshot);

                if push_opacity {
                    snapshot_in.pop();
                }
            }

            if let Some(cursor) = state.cursor()
                && let Some(grid) = state.grids.get(state.cursor_grid)
            {
                snapshot_cursor(
                    snapshot_in,
                    cursor,
                    font_ctx,
                    mono_ctx,
                    grid,
                    hl,
                    transparency,
                );
            }
        } else {
            self.snapshot_initializing(snapshot_in, &render_state);
        }
    }
}

impl NvimViewportObject {
    fn snapshot_initializing(&self, snapshot: &gtk::Snapshot, render_state: &RenderState) {
        let obj = self.obj();
        let layout = obj.create_pango_layout(Some("Loading…"));

        let attr_list = pango::AttrList::new();
        attr_list.insert(render_state.hl.fg().to_pango_fg());
        layout.set_attributes(Some(&attr_list));

        let (width, height) = layout.pixel_size();
        snapshot.render_layout(
            &obj.style_context(),
            obj.allocated_width() as f64 / 2.0 - width as f64 / 2.0,
            obj.allocated_height() as f64 / 2.0 - height as f64 / 2.0,
            &layout,
        );
    }
}
