use rebecca_core::environment::MapEnvironment;
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
fn missing_variable_returns_no_candidate() {
    let env = MapEnvironment::new();
    let template = PathTemplate::new("%LOCALAPPDATA%/Temp");

    let path = expand_template(&template, &env).expect("missing variable is not a hard error");

    assert!(path.is_none());
}

#[test]
fn unterminated_variable_is_invalid() {
    let env = MapEnvironment::new().with_var("TEMP", "C:/Temp");
    let template = PathTemplate::new("%TEMP/Cache");

    let err = expand_template(&template, &env).unwrap_err();

    assert!(err.to_string().contains("unterminated variable"));
}
