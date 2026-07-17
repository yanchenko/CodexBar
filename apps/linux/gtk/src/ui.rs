//! GTK4 main loop: status window + snapshot push + tray command channel.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::ffi;
use crate::tray::{self, AgentBarTray};

pub const APP_ID: &str = "app.agentbar.AgentBar";

pub fn run() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        let _ = ffi::engine_start();
    });

    let tray_handle: Rc<RefCell<Option<ksni::blocking::Handle<AgentBarTray>>>> =
        Rc::new(RefCell::new(None));
    let status_join: Rc<RefCell<Option<std::thread::JoinHandle<()>>>> =
        Rc::new(RefCell::new(None));

    app.connect_activate({
        let tray_handle = tray_handle.clone();
        let status_join = status_join.clone();
        move |app| on_activate(app, tray_handle.clone(), status_join.clone())
    });

    app.connect_shutdown(move |_| {
        if let Some(h) = status_join.borrow_mut().take() {
            // Status thread exits when channel closes / engine stops.
            let _ = h.join();
        }
        // Drop tray handle (ends DBus thread).
        *tray_handle.borrow_mut() = None;
        let _ = ffi::engine_stop();
    });

    app.run()
}

fn on_activate(
    app: &gtk::Application,
    tray_handle: Rc<RefCell<Option<ksni::blocking::Handle<AgentBarTray>>>>,
    status_join: Rc<RefCell<Option<std::thread::JoinHandle<()>>>>,
) {
    if let Some(existing) = app.windows().first() {
        existing.present();
        return;
    }

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("AgentBar")
        .default_width(420)
        .default_height(320)
        .build();

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_margin_top(12);
    list.set_margin_bottom(12);
    list.set_margin_start(12);
    list.set_margin_end(12);

    let header = gtk::Label::new(Some("AgentBar — provider usage"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);

    let version = gtk::Label::new(Some(&format!("Version {}", ffi::version())));
    version.set_halign(gtk::Align::Start);
    version.add_css_class("dim-label");

    let config = gtk::Label::new(Some(&format!("Config: {}", ffi::config_path())));
    config.set_halign(gtk::Align::Start);
    config.set_wrap(true);
    config.add_css_class("dim-label");

    let note = gtk::Label::new(Some(
        "Linux GTK4 host over ab-core. Tray uses StatusNotifierItem (ksni). If no SNI host is present, this window still works.",
    ));
    note.set_wrap(true);
    note.set_halign(gtk::Align::Start);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);
    vbox.append(&header);
    vbox.append(&version);
    vbox.append(&config);
    vbox.append(&note);
    vbox.append(&list);

    let refresh_btn = gtk::Button::with_label("Refresh Now");
    refresh_btn.connect_clicked(|_| {
        let _ = ffi::refresh_now();
    });
    vbox.append(&refresh_btn);

    window.set_child(Some(&vbox));

    // Stay alive in the tray: hide on close.
    let _hold = app.hold();
    window.connect_close_request(|w| {
        w.set_visible(false);
        glib::Propagation::Stop
    });

    // Tray on dedicated DBus thread; fail-soft if no session bus / SNI host.
    let (cmd_tx, cmd_rx) = async_channel::unbounded::<tray::Cmd>();
    {
        use ksni::blocking::TrayMethods;
        match AgentBarTray::new(cmd_tx.clone()).spawn() {
            Ok(handle) => {
                *tray_handle.borrow_mut() = Some(handle);
            }
            Err(e) => {
                eprintln!("agentbar-gtk: tray unavailable (SNI missing?): {e}");
            }
        }
    }

    // Snapshot push thread → GTK main loop.
    let (snap_tx, snap_rx) = async_channel::bounded::<String>(1);
    let join = std::thread::Builder::new()
        .name("ab-snapshot".into())
        .spawn(move || {
            let mut seq = 0u64;
            loop {
                let json = ffi::snapshot_wait(seq, 2_000);
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                    if let Some(s) = v.get("seq").and_then(|x| x.as_u64()) {
                        seq = s;
                    }
                }
                if snap_tx.send_blocking(json).is_err() {
                    break;
                }
            }
        })
        .ok();
    *status_join.borrow_mut() = join;

    let list_rc = list.clone();
    let tray_handle_ui = tray_handle.clone();
    glib::spawn_future_local(async move {
        while let Ok(json) = snap_rx.recv().await {
            let lines = tray_lines_from_snapshot(&json);
            while let Some(row) = list_rc.row_at_index(0) {
                list_rc.remove(&row);
            }
            for line in &lines {
                let row = gtk::Label::new(Some(line));
                row.set_halign(gtk::Align::Start);
                row.set_wrap(true);
                list_rc.append(&row);
            }
            if let Some(handle) = tray_handle_ui.borrow().as_ref() {
                let lines2 = lines.clone();
                let _ = handle.update(|t: &mut AgentBarTray| {
                    t.lines = lines2;
                });
            }
        }
    });

    let window_rc = window.clone();
    let app_rc = app.clone();
    glib::spawn_future_local(async move {
        while let Ok(cmd) = cmd_rx.recv().await {
            match cmd {
                tray::Cmd::ShowWindow => {
                    ffi::note_menu_opened();
                    window_rc.set_visible(true);
                    window_rc.present();
                }
                tray::Cmd::Refresh => {
                    let _ = ffi::refresh_now();
                }
                tray::Cmd::Quit => {
                    app_rc.quit();
                }
            }
        }
    });

    // Show window once so first-run has a surface even without tray.
    window.present();
}

fn tray_lines_from_snapshot(json: &str) -> Vec<String> {
    let Ok(snap) = serde_json::from_str::<ab_model::UsageSnapshot>(json) else {
        return vec!["Waiting for snapshot…".into()];
    };
    if snap.providers.is_empty() {
        return vec!["No providers enabled".into()];
    }
    snap.providers
        .iter()
        .map(|p| {
            let name = {
                let mut c = p.id.chars();
                match c.next() {
                    None => p.id.clone(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            };
            if let Some(err) = &p.error {
                let code = p
                    .error_code
                    .as_deref()
                    .map(|c| format!(" ({c})"))
                    .unwrap_or_default();
                return format!("{name}: {err}{code}");
            }
            if let Some(w) = &p.primary {
                let mut s = format!("{name}: {:.0}%", w.used_percent);
                if let Some(label) = &p.account_label {
                    s.push_str(" · ");
                    s.push_str(label);
                }
                return s;
            }
            if let Some(c) = p.credits_remaining {
                return format!("{name}: {c:.2} credits");
            }
            format!("{name}: —")
        })
        .collect()
}
