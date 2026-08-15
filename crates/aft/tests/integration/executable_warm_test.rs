/// Guard the fixture-installation sweep against accidental raw executable
/// writes. The listed sources contain every test helper that creates a script
/// or copies a binary for later execution, so each needs a warm call in setup.
#[test]
fn executable_fixture_installers_warm_new_inodes() {
    let installers = [
        (
            "backups_dual_write_test.rs",
            include_str!("backups_dual_write_test.rs"),
            3,
        ),
        ("bash_test.rs", include_str!("bash_test.rs"), 1),
        ("configure_test.rs", include_str!("configure_test.rs"), 1),
        ("edit_test.rs", include_str!("edit_test.rs"), 1),
        ("format_test.rs", include_str!("format_test.rs"), 5),
        ("grep_glob_test.rs", include_str!("grep_glob_test.rs"), 1),
        (
            "lsp_diagnostics_test.rs",
            include_str!("lsp_diagnostics_test.rs"),
            1,
        ),
        (
            "lsp_inspect_test.rs",
            include_str!("lsp_inspect_test.rs"),
            1,
        ),
        (
            "lsp_manager_test.rs",
            include_str!("lsp_manager_test.rs"),
            1,
        ),
    ];

    for (source_name, source, expected_warms) in installers {
        let warm_calls = source.matches("warm_executable(").count();
        assert!(
            warm_calls >= expected_warms,
            "{source_name} has {warm_calls} warming calls; expected at least {expected_warms}"
        );
    }
}
