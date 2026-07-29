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
    pub struct SpoolTray {
        status: String,
    }

    impl ksni::Tray for SpoolTray {
        fn id(&self) -> String {
            "spool".into()
        }

        fn title(&self) -> String {
            "Spool".into()
        }

        fn icon_name(&self) -> String {
            "printer".into()
        }

        fn menu_about_to_show(&mut self) {
            self.status = fetch_status().unwrap_or_else(|| "Agent unavailable".into());
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::{MenuItem, StandardItem};
            vec![
                StandardItem {
                    label: self.status.clone(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: "Open Spool".into(),
                    activate: Box::new(|_| {
                        let _ = Command::new("xdg-open")
                            .arg("http://127.0.0.1:39100")
                            .spawn();
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let tray = SpoolTray {
            status: fetch_status().unwrap_or_else(|| "Agent unavailable".into()),
        };
        let _handle = tray.spawn()?;
        loop {
            std::thread::park();
        }
    }

    fn fetch_status() -> Option<String> {
        let token_path = std::env::var_os("SPOOL_DATA_DIR")
            .map_or_else(|| PathBuf::from(".spool"), PathBuf::from)
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
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("Spool Linux shell failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("spool-shell-linux is only available on Linux");
}
