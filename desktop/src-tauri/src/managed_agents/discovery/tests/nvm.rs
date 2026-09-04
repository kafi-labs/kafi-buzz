use super::super::{
    find_nvm_default_bin, is_login_shell_path_uninit, is_safe_nvm_tag, login_shell_path,
    parse_semver_tag, refresh_login_shell_path,
};

#[test]
fn parse_semver_tag_accepts_plain_version() {
    assert_eq!(parse_semver_tag("v20.11.1"), Some((20, 11, 1)));
}

#[test]
fn parse_semver_tag_accepts_prerelease_suffix() {
    assert_eq!(parse_semver_tag("v18.0.0-rc1"), Some((18, 0, 0)));
}

#[test]
fn parse_semver_tag_rejects_invalid_values() {
    assert_eq!(parse_semver_tag("20.11.1"), None);
    assert_eq!(parse_semver_tag("vX.Y.Z"), None);
}

#[test]
fn semver_ordering_chooses_highest() {
    let mut tags = [
        parse_semver_tag("v16.0.0").unwrap(),
        parse_semver_tag("v20.11.1").unwrap(),
        parse_semver_tag("v18.12.0").unwrap(),
    ];
    tags.sort();
    assert_eq!(tags.last(), parse_semver_tag("v20.11.1").as_ref());
}

#[cfg(unix)]
fn make_nvm_version_dir(home: &std::path::Path, tag: &str) {
    std::fs::create_dir_all(home.join(".nvm/versions/node").join(tag).join("bin")).unwrap();
}

#[cfg(unix)]
fn write_alias(home: &std::path::Path, name: &str, content: &str) {
    let alias_dir = home.join(".nvm/alias");
    std::fs::create_dir_all(&alias_dir).unwrap();
    std::fs::write(alias_dir.join(name), content).unwrap();
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_returns_dir_from_alias_default() {
    let home = tempfile::tempdir().unwrap();
    make_nvm_version_dir(home.path(), "v20.11.1");
    write_alias(home.path(), "default", "v20.11.1\n");
    assert_eq!(
        find_nvm_default_bin(home.path()),
        Some(home.path().join(".nvm/versions/node/v20.11.1/bin"))
    );
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_follows_one_alias_hop() {
    let home = tempfile::tempdir().unwrap();
    make_nvm_version_dir(home.path(), "v18.0.0");
    write_alias(home.path(), "default", "lts/hydrogen\n");
    let alias_dir = home.path().join(".nvm/alias/lts");
    std::fs::create_dir_all(&alias_dir).unwrap();
    std::fs::write(alias_dir.join("hydrogen"), "v18.0.0\n").unwrap();
    assert_eq!(
        find_nvm_default_bin(home.path()),
        Some(home.path().join(".nvm/versions/node/v18.0.0/bin"))
    );
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_falls_back_to_highest_semver() {
    let home = tempfile::tempdir().unwrap();
    for tag in ["v16.0.0", "v20.11.1", "v18.12.0"] {
        make_nvm_version_dir(home.path(), tag);
    }
    assert_eq!(
        find_nvm_default_bin(home.path()),
        Some(home.path().join(".nvm/versions/node/v20.11.1/bin"))
    );
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_returns_none_without_versions() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(find_nvm_default_bin(home.path()), None);
    std::fs::create_dir_all(home.path().join(".nvm/versions/node")).unwrap();
    assert_eq!(find_nvm_default_bin(home.path()), None);
}

#[test]
fn refresh_login_shell_path_clears_cache() {
    let _guard = crate::managed_agents::lock_path_mutex();
    let before = login_shell_path();
    assert!(!is_login_shell_path_uninit());
    refresh_login_shell_path();
    assert!(is_login_shell_path_uninit());
    assert_eq!(before, login_shell_path());
    assert!(!is_login_shell_path_uninit());
}

#[test]
fn is_safe_nvm_tag_accepts_expected_tags() {
    for tag in ["v20.11.1", "v18.0.0-rc1", "lts/hydrogen", "v22.1.0"] {
        assert!(is_safe_nvm_tag(tag));
    }
}

#[test]
fn is_safe_nvm_tag_rejects_unsafe_tags() {
    for tag in [
        "/tmp/evil",
        "/usr/local/bin",
        "/",
        "../../../etc/passwd",
        "v20.11.1/../../../etc",
        "..",
        "v20.11.1; rm -rf ~",
        "v20.11.1\n/tmp/evil",
        "v20.11.1\0",
        "$(evil)",
        "",
    ] {
        assert!(!is_safe_nvm_tag(tag));
    }
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_rejects_unsafe_aliases() {
    for alias in ["/tmp/evil\n", "../../etc/passwd\n"] {
        let home = tempfile::tempdir().unwrap();
        write_alias(home.path(), "default", alias);
        assert_eq!(find_nvm_default_bin(home.path()), None);
    }
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_rejects_absolute_hop_tag() {
    let home = tempfile::tempdir().unwrap();
    write_alias(home.path(), "default", "lts/testing\n");
    let lts_dir = home.path().join(".nvm/alias/lts");
    std::fs::create_dir_all(&lts_dir).unwrap();
    std::fs::write(lts_dir.join("testing"), "/tmp/evil\n").unwrap();
    assert_eq!(find_nvm_default_bin(home.path()), None);
}
