use rebecca_core::Platform;
use rebecca_core::environment::{MapEnvironment, PlatformEnvironment};
use rebecca_core::path_template::{PathTemplate, expand_template};

#[test]
fn expands_percent_variables_from_injected_environment() {
    let env = MapEnvironment::new().with_var("LOCALAPPDATA", "C:/Users/Alice/AppData/Local");
    let template = PathTemplate::new("%LOCALAPPDATA%/Temp");

    let path = expand_template(&template, &env)
        .expect("template should expand")
        .expect("variable should be present");

    assert_eq!(
        path.to_string_lossy().replace('\\', "/"),
        "C:/Users/Alice/AppData/Local/Temp"
    );
}

#[test]
fn expands_backslash_separators_as_native_path_segments() {
    let env = MapEnvironment::new().with_var("LOCALAPPDATA", "C:/Users/Alice/AppData/Local");
    let template = PathTemplate::new("%LOCALAPPDATA%\\npm-cache\\_cacache");

    let path = expand_template(&template, &env)
        .expect("template should expand")
        .expect("variable should be present");

    assert_eq!(
        normalized(path),
        "C:/Users/Alice/AppData/Local/npm-cache/_cacache"
    );
}

#[test]
fn missing_variable_returns_no_candidate() {
    let env = MapEnvironment::new();
    let template = PathTemplate::new("%LOCALAPPDATA%/Temp");

    let path = expand_template(&template, &env).expect("missing variable is not a hard error");

    assert!(path.is_none());
}

#[test]
fn linux_xdg_cache_home_falls_back_to_home_cache_when_unset() {
    let env = PlatformEnvironment::new(
        Platform::Linux,
        MapEnvironment::new().with_var("HOME", "/home/alice"),
    );
    let template = PathTemplate::new("%XDG_CACHE_HOME%/pip");

    let path = expand_template(&template, &env)
        .expect("template should expand")
        .expect("XDG_CACHE_HOME should be synthesized from HOME");

    assert_eq!(normalized(path), "/home/alice/.cache/pip");
}

#[test]
fn linux_explicit_xdg_cache_home_wins() {
    let env = PlatformEnvironment::new(
        Platform::Linux,
        MapEnvironment::new()
            .with_var("HOME", "/home/alice")
            .with_var("XDG_CACHE_HOME", "/mnt/cache/alice"),
    );
    let template = PathTemplate::new("%XDG_CACHE_HOME%/pip");

    let path = expand_template(&template, &env)
        .expect("template should expand")
        .expect("explicit XDG_CACHE_HOME should be present");

    assert_eq!(normalized(path), "/mnt/cache/alice/pip");
}

#[test]
fn linux_missing_home_keeps_xdg_candidate_absent() {
    let env = PlatformEnvironment::new(Platform::Linux, MapEnvironment::new());
    let template = PathTemplate::new("%XDG_CACHE_HOME%/pip");

    let path = expand_template(&template, &env).expect("missing HOME is not a hard error");

    assert!(path.is_none());
}

#[test]
fn linux_tmpdir_is_not_synthesized() {
    let env = PlatformEnvironment::new(
        Platform::Linux,
        MapEnvironment::new().with_var("HOME", "/home/alice"),
    );
    let template = PathTemplate::new("%TMPDIR%/rebecca");

    let path = expand_template(&template, &env).expect("missing TMPDIR is not a hard error");

    assert!(path.is_none());
}

#[test]
fn non_linux_platform_environment_does_not_synthesize_xdg_defaults() {
    let env = PlatformEnvironment::new(
        Platform::Windows,
        MapEnvironment::new().with_var("HOME", "/home/alice"),
    );
    let template = PathTemplate::new("%XDG_CACHE_HOME%/pip");

    let path =
        expand_template(&template, &env).expect("missing XDG_CACHE_HOME is not a hard error");

    assert!(path.is_none());
}

#[test]
fn unterminated_variable_is_invalid() {
    let env = MapEnvironment::new().with_var("TEMP", "C:/Temp");
    let template = PathTemplate::new("%TEMP/Cache");

    let err = expand_template(&template, &env).unwrap_err();

    assert!(err.to_string().contains("unterminated variable"));
}

fn normalized(path: std::path::PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}
