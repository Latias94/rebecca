pub(crate) fn default_home_suffix(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "MACOS_CACHE_HOME" => Some(&["Library", "Caches"]),
        "MACOS_APPLICATION_SUPPORT_HOME" => Some(&["Library", "Application Support"]),
        "MACOS_LOG_HOME" => Some(&["Library", "Logs"]),
        "MACOS_CONTAINER_HOME" => Some(&["Library", "Containers"]),
        "MACOS_GROUP_CONTAINER_HOME" => Some(&["Library", "Group Containers"]),
        _ => None,
    }
}

pub(crate) fn cache_home_tail_start(segments: &[&str]) -> Option<usize> {
    find_segment(segments, "%macos_cache_home%")
        .map(|index| index + 1)
        .or_else(|| user_library_tail_start(segments, &["caches"]))
}

pub(crate) fn application_support_home_tail_start(segments: &[&str]) -> Option<usize> {
    find_segment(segments, "%macos_application_support_home%")
        .map(|index| index + 1)
        .or_else(|| user_library_tail_start(segments, &["application support"]))
}

pub(crate) fn log_home_tail_start(segments: &[&str]) -> Option<usize> {
    find_segment(segments, "%macos_log_home%")
        .map(|index| index + 1)
        .or_else(|| user_library_tail_start(segments, &["logs"]))
}

pub(crate) fn chromium_profile_tail_start(segments: &[&str]) -> Option<usize> {
    let start = application_support_home_tail_start(segments)?;
    let tail = segments.get(start..)?;

    if tail.starts_with(&["google", "chrome"]) {
        return Some(start + 2);
    }
    if tail.starts_with(&["bravesoftware", "brave-browser"]) {
        return Some(start + 2);
    }
    if tail
        .first()
        .is_some_and(|app| matches!(*app, "chromium" | "microsoft edge"))
    {
        return Some(start + 1);
    }

    None
}

pub(crate) fn is_durable_data_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["library", "application support"])
        || has_sequence(segments, &["library", "caches"])
        || has_sequence(segments, &["library", "containers"])
        || has_sequence(segments, &["library", "group containers"])
        || has_sequence(segments, &["library", "keychains"])
        || has_sequence(segments, &["library", "mail"])
        || has_sequence(segments, &["library", "messages"])
        || has_sequence(segments, &["library", "photos"])
        || has_sequence(segments, &["library", "preferences"])
        || has_sequence(segments, &["library", "safari"])
        || find_segment(segments, "%macos_cache_home%").is_some()
        || find_segment(segments, "%macos_application_support_home%").is_some()
        || find_segment(segments, "%macos_container_home%").is_some()
        || find_segment(segments, "%macos_group_container_home%").is_some()
}

fn user_library_tail_start(segments: &[&str], library_child: &[&str]) -> Option<usize> {
    if library_child.is_empty() {
        return None;
    }

    let required_len = 3 + library_child.len();
    if segments.len() < required_len {
        return None;
    }

    segments
        .windows(required_len)
        .position(|window| {
            window[0] == "users"
                && !window[1].is_empty()
                && window[1] != "shared"
                && window[2] == "library"
                && &window[3..] == library_child
        })
        .map(|index| index + required_len)
}

fn has_sequence(segments: &[&str], sequence: &[&str]) -> bool {
    if sequence.is_empty() || segments.len() < sequence.len() {
        return false;
    }

    segments
        .windows(sequence.len())
        .any(|window| window == sequence)
}

fn find_segment(segments: &[&str], needle: &str) -> Option<usize> {
    segments.iter().position(|segment| *segment == needle)
}
