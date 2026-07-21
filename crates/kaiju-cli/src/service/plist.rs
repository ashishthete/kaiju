//! Renders a launchd agent description into plist XML.
//!
//! Pure: no filesystem, no process, no launchd. This is the only part of the
//! service module with enough logic to be worth unit-testing on its own.

use std::path::PathBuf;

/// One launchd user agent.
pub struct ServiceSpec {
    pub label: String,
    pub program: PathBuf,
    pub run_at_load: bool,
    pub keep_alive: bool,
    pub stdout_path: Option<PathBuf>,
    pub stderr_path: Option<PathBuf>,
    /// Ordered so the rendered plist is deterministic.
    pub environment: Vec<(String, String)>,
}

pub fn to_plist_xml(spec: &ServiceSpec) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );

    push_string(&mut out, "Label", &spec.label);
    push_string(&mut out, "Program", &spec.program.display().to_string());
    push_bool(&mut out, "RunAtLoad", spec.run_at_load);
    push_bool(&mut out, "KeepAlive", spec.keep_alive);

    if let Some(path) = &spec.stdout_path {
        push_string(&mut out, "StandardOutPath", &path.display().to_string());
    }
    if let Some(path) = &spec.stderr_path {
        push_string(&mut out, "StandardErrorPath", &path.display().to_string());
    }

    if !spec.environment.is_empty() {
        out.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
        for (key, value) in &spec.environment {
            out.push_str(&format!(
                "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
                xml_escape(key),
                xml_escape(value)
            ));
        }
        out.push_str("\t</dict>\n");
    }

    out.push_str("</dict>\n</plist>\n");
    out
}

fn push_string(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!(
        "\t<key>{}</key>\n\t<string>{}</string>\n",
        xml_escape(key),
        xml_escape(value)
    ));
}

fn push_bool(out: &mut String, key: &str, value: bool) {
    let tag = if value { "<true/>" } else { "<false/>" };
    out.push_str(&format!("\t<key>{}</key>\n\t{}\n", xml_escape(key), tag));
}

/// `&` must be replaced first, or the `&` in the later entities double-escapes.
fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daemon_fixture() -> ServiceSpec {
        ServiceSpec {
            label: "dev.kaiju.daemon".to_string(),
            program: PathBuf::from("/Users/x/.cargo/bin/kaiju-daemon"),
            run_at_load: true,
            keep_alive: true,
            stdout_path: Some(PathBuf::from("/Users/x/.kaiju/logs/daemon.out.log")),
            stderr_path: Some(PathBuf::from("/Users/x/.kaiju/logs/daemon.err.log")),
            environment: vec![("PATH".to_string(), "/opt/homebrew/bin:/usr/bin".to_string())],
        }
    }

    fn pet_fixture() -> ServiceSpec {
        ServiceSpec {
            label: "dev.kaiju.pet".to_string(),
            program: PathBuf::from("/Users/x/.cargo/bin/kaiju-pet-shell"),
            run_at_load: true,
            keep_alive: false,
            stdout_path: None,
            stderr_path: None,
            environment: vec![],
        }
    }

    #[test]
    fn plist_has_xml_header_and_root() {
        let xml = to_plist_xml(&daemon_fixture());
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<!DOCTYPE plist PUBLIC"));
        assert!(xml.contains("<plist version=\"1.0\">"));
        assert!(xml.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn plist_contains_label_and_program() {
        let xml = to_plist_xml(&daemon_fixture());
        assert!(xml.contains("<key>Label</key>\n\t<string>dev.kaiju.daemon</string>"));
        assert!(
            xml.contains("<key>Program</key>\n\t<string>/Users/x/.cargo/bin/kaiju-daemon</string>")
        );
    }

    #[test]
    fn daemon_keeps_alive_and_pet_does_not() {
        let daemon = to_plist_xml(&daemon_fixture());
        assert!(daemon.contains("<key>KeepAlive</key>\n\t<true/>"));

        let pet = to_plist_xml(&pet_fixture());
        assert!(pet.contains("<key>KeepAlive</key>\n\t<false/>"));
    }

    #[test]
    fn both_run_at_load() {
        assert!(to_plist_xml(&daemon_fixture()).contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(to_plist_xml(&pet_fixture()).contains("<key>RunAtLoad</key>\n\t<true/>"));
    }

    #[test]
    fn path_environment_is_embedded() {
        // The whole point of the feature: launchd's default PATH omits
        // /opt/homebrew/bin, so the daemon could not find tmux or the agent CLIs.
        let xml = to_plist_xml(&daemon_fixture());
        assert!(xml.contains("<key>EnvironmentVariables</key>"));
        assert!(xml.contains("<key>PATH</key>\n\t\t<string>/opt/homebrew/bin:/usr/bin</string>"));
    }

    #[test]
    fn empty_environment_omits_the_dict_entirely() {
        let xml = to_plist_xml(&pet_fixture());
        assert!(!xml.contains("EnvironmentVariables"));
    }

    #[test]
    fn log_paths_present_for_daemon_absent_for_pet() {
        let daemon = to_plist_xml(&daemon_fixture());
        assert!(daemon.contains("<key>StandardOutPath</key>"));
        assert!(daemon.contains("/Users/x/.kaiju/logs/daemon.out.log"));
        assert!(daemon.contains("<key>StandardErrorPath</key>"));
        assert!(daemon.contains("/Users/x/.kaiju/logs/daemon.err.log"));

        let pet = to_plist_xml(&pet_fixture());
        assert!(!pet.contains("StandardOutPath"));
        assert!(!pet.contains("StandardErrorPath"));
    }

    #[test]
    fn special_characters_are_escaped() {
        let spec = ServiceSpec {
            label: "dev.kaiju.daemon".to_string(),
            program: PathBuf::from("/Users/a & b/bin/<kaiju>"),
            run_at_load: true,
            keep_alive: false,
            stdout_path: None,
            stderr_path: None,
            environment: vec![],
        };
        let xml = to_plist_xml(&spec);
        assert!(xml.contains("/Users/a &amp; b/bin/&lt;kaiju&gt;"));
        assert!(!xml.contains("<kaiju>"));
    }
}
