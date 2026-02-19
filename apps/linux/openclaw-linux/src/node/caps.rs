pub fn current_caps(canvas_enabled: bool) -> Vec<String> {
    let mut caps = vec!["canvas".to_string(), "screen".to_string()];
    if !canvas_enabled {
        caps.retain(|cap| cap != "canvas");
    }
    caps
}

pub fn current_commands(canvas_enabled: bool) -> Vec<String> {
    let mut commands = Vec::new();
    if canvas_enabled {
        commands.extend(
            ["canvas.present", "canvas.hide", "canvas.navigate"]
                .iter()
                .map(|s| s.to_string()),
        );
    }

    // TODO: add camera/location/screen/system commands as they are implemented.
    commands
}

#[cfg(test)]
mod tests {
    use super::{current_caps, current_commands};

    #[test]
    fn canvas_caps_are_removed_when_disabled() {
        let caps = current_caps(false);
        assert!(!caps.iter().any(|cap| cap == "canvas"));
    }

    #[test]
    fn only_implemented_canvas_commands_are_advertised() {
        let commands = current_commands(true);
        assert_eq!(
            commands,
            vec![
                "canvas.present".to_string(),
                "canvas.hide".to_string(),
                "canvas.navigate".to_string(),
            ]
        );
    }
}
