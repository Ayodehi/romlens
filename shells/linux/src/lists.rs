//! A list of Rust values in a `GtkListView`: the model, the row factory and
//! the activation signal, so the navigator and the results pane share one way
//! to put rows on screen.

use gtk::prelude::*;
use gtk::{gio, glib};

/// Build a list over `T`. `setup` makes an empty row, `bind` fills it from a
/// value, and `activate` runs when a row is clicked or Enter is pressed.
pub struct ValueList<T: 'static> {
    pub widget: gtk::ScrolledWindow,
    store: gio::ListStore,
    selection: Option<gtk::SingleSelection>,
    _t: std::marker::PhantomData<T>,
}

impl<T: 'static> ValueList<T> {
    /// `selectable` keeps a highlighted row, for lists that show a current
    /// item (find hits, references); the navigator's rows are plain buttons.
    pub fn new(
        selectable: bool,
        setup: impl Fn() -> gtk::Widget + 'static,
        bind: impl Fn(&gtk::Widget, &T) + 'static,
        activate: impl Fn(u32, &T) + 'static,
    ) -> Self {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                item.set_child(Some(&setup()));
            }
        });
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let (Some(child), Some(obj)) = (
                item.child(),
                item.item().and_downcast::<glib::BoxedAnyObject>(),
            ) else {
                return;
            };
            bind(&child, &obj.borrow::<T>());
        });
        let single = selectable.then(|| {
            let s = gtk::SingleSelection::new(Some(store.clone()));
            s.set_autoselect(false);
            s.set_can_unselect(true);
            s.set_selected(gtk::INVALID_LIST_POSITION);
            s
        });
        let model: gtk::SelectionModel = match &single {
            Some(s) => s.clone().upcast(),
            None => gtk::NoSelection::new(Some(store.clone())).upcast(),
        };
        let view = gtk::ListView::new(Some(model), Some(factory));
        view.set_single_click_activate(true);
        view.add_css_class("navigation-sidebar");
        let model = store.clone();
        view.connect_activate(move |_, position| {
            if let Some(obj) = model.item(position).and_downcast::<glib::BoxedAnyObject>() {
                activate(position, &obj.borrow::<T>());
            }
        });
        let widget = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&view)
            .build();
        Self {
            widget,
            store,
            selection: single,
            _t: std::marker::PhantomData,
        }
    }

    /// Highlight the current row, or none.
    pub fn set_current(&self, position: Option<u32>) {
        if let Some(s) = &self.selection {
            s.set_selected(position.unwrap_or(gtk::INVALID_LIST_POSITION));
        }
    }

    /// Replace every row.
    pub fn set(&self, items: impl IntoIterator<Item = T>) {
        let objects: Vec<_> = items.into_iter().map(glib::BoxedAnyObject::new).collect();
        self.store.splice(0, self.store.n_items(), &objects);
    }
}

/// A row: a horizontal box with the given margins, ready for children.
pub fn row_box() -> gtk::Box {
    gtk::Box::builder()
        .spacing(8)
        .margin_start(6)
        .margin_end(6)
        .margin_top(5)
        .margin_bottom(5)
        .build()
}

/// A flexible gap that pushes what follows to the end of the row.
pub fn spacer() -> gtk::Box {
    gtk::Box::builder().hexpand(true).build()
}

/// The `n`th child of a row built by `row_box`.
pub fn child_at(row: &gtk::Widget, n: usize) -> gtk::Widget {
    let mut w = row.first_child();
    for _ in 0..n {
        w = w.and_then(|c| c.next_sibling());
    }
    w.expect("row child")
}
