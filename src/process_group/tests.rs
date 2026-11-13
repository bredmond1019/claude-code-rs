use super::*;

#[test]
fn default_grace_resolves_to_five_seconds() {
    assert_eq!(DEFAULT_GROUP_KILL_GRACE, Duration::from_secs(5));
    assert_eq!(
        effective_group_kill_grace(&Config::default()),
        Duration::from_secs(5)
    );
}

#[test]
fn override_grace_is_honoured() {
    let config = Config {
        group_kill_grace: Some(Duration::from_millis(250)),
        ..Config::default()
    };
    assert_eq!(
        effective_group_kill_grace(&config),
        Duration::from_millis(250)
    );
}
