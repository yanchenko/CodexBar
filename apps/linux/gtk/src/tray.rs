//! StatusNotifierItem tray on a dedicated DBus thread (ksni blocking).
//! Menu callbacks only enqueue work for the GTK main loop — never touch GTK here.

use ksni::menu::{MenuItem, StandardItem};
use ksni::Tray;

/// Tray menu → GTK main loop.
#[derive(Debug, Clone)]
pub enum Cmd {
    ShowWindow,
    Refresh,
    Quit,
}

pub struct AgentBarTray {
    pub tooltip: String,
    pub lines: Vec<String>,
    tx: async_channel::Sender<Cmd>,
}

impl AgentBarTray {
    pub fn new(tx: async_channel::Sender<Cmd>) -> Self {
        Self {
            tooltip: "AgentBar".into(),
            lines: vec!["Starting…".into()],
            tx,
        }
    }
}

impl Tray for AgentBarTray {
    fn id(&self) -> String {
        crate::ui::APP_ID.into()
    }

    fn title(&self) -> String {
        "AgentBar".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let body = if self.lines.is_empty() {
            self.tooltip.clone()
        } else {
            self.lines.join("\n")
        };
        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
            title: "AgentBar".into(),
            description: body,
        }
    }

    fn icon_name(&self) -> String {
        // Theme icon; falls back to text label on hosts without the name.
        "utilities-system-monitor".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.try_send(Cmd::ShowWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = Vec::new();
        for line in &self.lines {
            items.push(
                StandardItem {
                    label: line.clone(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(MenuItem::Separator);
        let (refresh, show, quit) = (self.tx.clone(), self.tx.clone(), self.tx.clone());
        items.push(
            StandardItem {
                label: "Refresh Now".into(),
                icon_name: "view-refresh-symbolic".into(),
                activate: Box::new(move |_| {
                    let _ = refresh.try_send(Cmd::Refresh);
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Settings".into(),
                icon_name: "preferences-system-symbolic".into(),
                activate: Box::new(move |_| {
                    let _ = show.try_send(Cmd::ShowWindow);
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(move |_| {
                    let _ = quit.try_send(Cmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}
