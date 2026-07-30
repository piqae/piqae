#[cfg(target_os = "linux")]
mod linux {
    use ksni::blocking::TrayMethods;
    use std::{
        io::{Read as _, Write as _},
        net::TcpStream,
        path::PathBuf,
        process::Command,
        time::Duration,
    };

    #[derive(Debug)]
    pub struct PiqaeTray {
        status: String,
    }

    impl ksni::Tray for PiqaeTray {
        fn id(&self) -> String {
            "piqae".into()
        }

        fn title(&self) -> String {
            "Piqae".into()
        }

        fn icon_name(&self) -> String {
            "printer".into()
        }

        fn menu_about_to_show(&mut self) {
            self.status = fetch_status().unwrap_or_else(|| "Agent unavailable".into());
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::{MenuItem, StandardItem};
            let mut items = vec![
                StandardItem {
                    label: self.status.clone(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            ];
            if let Some(dashboard_url) = dashboard_url() {
                items.push(MenuItem::Separator);
                items.push(
                    StandardItem {
                        label: "Open Piqae".into(),
                        activate: Box::new(move |_| {
                            let _ = Command::new("xdg-open").arg(&dashboard_url).spawn();
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            items
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let tray = PiqaeTray {
            status: fetch_status().unwrap_or_else(|| "Agent unavailable".into()),
        };
        let _handle = tray.spawn()?;
        loop {
            std::thread::park();
        }
    }

    fn fetch_status() -> Option<String> {
        let token_path = std::env::var_os("PIQAE_DATA_DIR")
            .map_or_else(|| PathBuf::from(".piqae"), PathBuf::from)
            .join("local.token");
        let token = std::fs::read_to_string(token_path).ok()?;
        let mut stream = TcpStream::connect_timeout(
            &"127.0.0.1:39100".parse().ok()?,
            Duration::from_millis(500),
        )
        .ok()?;
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .ok()?;
        write!(
            stream,
            "GET /v1/local/status HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
            token.trim()
        )
        .ok()?;
        let mut response = String::new();
        stream.read_to_string(&mut response).ok()?;
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| format!("Agent: {}", compact_state(body)))
    }

    fn compact_state(body: &str) -> &str {
        if body.contains("\"connection\":\"connected\"") {
            "Connected"
        } else if body.contains("\"connection\":\"local_only\"") {
            "Local only"
        } else if body.contains("\"connection\":\"degraded\"") {
            "Degraded"
        } else {
            "Offline"
        }
    }

    fn dashboard_url() -> Option<String> {
        let value = std::env::var("PIQAE_DASHBOARD_URL").ok()?;
        let value = value.trim();
        if value.contains(['\r', '\n'])
            || !(value.starts_with("https://") || value.starts_with("http://"))
        {
            return None;
        }
        Some(value.to_owned())
    }
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("Piqae Linux shell failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("piqae-shell-linux is only available on Linux");
}
