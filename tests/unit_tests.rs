//! Unit tests for jj-ryu modules

mod common;

mod analysis_test {
    use crate::common::{make_linear_stack, make_multi_bookmark_segment};
    use jj_ryu::error::Error;
    use jj_ryu::submit::{
        analyze_submission, generate_pr_title, get_base_branch, select_bookmark_for_segment,
    };

    #[test]
    fn test_analyze_middle_of_stack() {
        // Stack: a -> b -> c, target b
        // Should return [a, b] not [a, b, c]
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let result = analyze_submission(&graph, Some("feat-b")).unwrap();

        assert_eq!(result.target_bookmark, "feat-b");
        assert_eq!(result.segments.len(), 2);
        assert_eq!(result.segments[0].bookmark.name, "feat-a");
        assert_eq!(result.segments[1].bookmark.name, "feat-b");
    }

    #[test]
    fn test_analyze_root_of_stack() {
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let result = analyze_submission(&graph, Some("feat-a")).unwrap();

        assert_eq!(result.segments.len(), 1);
        assert_eq!(result.segments[0].bookmark.name, "feat-a");
    }

    #[test]
    fn test_analyze_leaf_of_stack() {
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let result = analyze_submission(&graph, Some("feat-c")).unwrap();

        assert_eq!(result.segments.len(), 3);
        assert_eq!(result.segments[2].bookmark.name, "feat-c");
    }

    #[test]
    fn test_get_base_branch_three_level_stack() {
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        assert_eq!(
            get_base_branch("feat-a", &analysis.segments, "main").unwrap(),
            "main"
        );
        assert_eq!(
            get_base_branch("feat-b", &analysis.segments, "main").unwrap(),
            "feat-a"
        );
        assert_eq!(
            get_base_branch("feat-c", &analysis.segments, "main").unwrap(),
            "feat-b"
        );
    }

    #[test]
    fn test_generate_pr_title_uses_root_commit_description() {
        // Fixture creates description "Commit for {name}"
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let title = generate_pr_title("feat-a", &analysis.segments).unwrap();
        // Should use the actual commit description, not just the bookmark name
        assert_eq!(title, "Commit for feat-a");
    }

    #[test]
    fn test_analyze_nonexistent_bookmark_error_type() {
        let graph = make_linear_stack(&["feat-a", "feat-b"]);
        let result = analyze_submission(&graph, Some("nonexistent"));

        // Verify we get the correct error type with the bookmark name
        match result {
            Err(Error::BookmarkNotFound(name)) => assert_eq!(name, "nonexistent"),
            other => panic!("Expected BookmarkNotFound error, got: {other:?}"),
        }
    }

    // === Multi-bookmark tests ===

    #[test]
    fn test_analyze_multi_bookmark_segment_selects_target() {
        // Two bookmarks pointing to the same commit
        let graph = make_multi_bookmark_segment(&["feat-a", "feat-b"]);
        let analysis = analyze_submission(&graph, Some("feat-b")).unwrap();

        // Should select the target bookmark
        assert_eq!(analysis.segments.len(), 1);
        assert_eq!(analysis.segments[0].bookmark.name, "feat-b");
    }

    #[test]
    fn test_select_bookmark_prefers_shorter_name() {
        let graph = make_multi_bookmark_segment(&["feature-auth", "auth"]);
        // Don't specify target - should prefer shorter name
        let segment = &graph.stack.as_ref().unwrap().segments[0];
        let selected = select_bookmark_for_segment(segment, None);
        assert_eq!(selected.name, "auth");
    }

    #[test]
    fn test_select_bookmark_filters_temporary() {
        let graph = make_multi_bookmark_segment(&["wip-feature", "feature"]);
        let segment = &graph.stack.as_ref().unwrap().segments[0];
        let selected = select_bookmark_for_segment(segment, None);
        // Should filter out "wip-feature" and select "feature"
        assert_eq!(selected.name, "feature");
    }

    #[test]
    fn test_select_bookmark_filters_temp_suffix() {
        let graph = make_multi_bookmark_segment(&["auth-old", "auth"]);
        let segment = &graph.stack.as_ref().unwrap().segments[0];
        let selected = select_bookmark_for_segment(segment, None);
        assert_eq!(selected.name, "auth");
    }

    #[test]
    fn test_select_bookmark_all_temporary_uses_shortest() {
        // When all bookmarks are temporary, still picks shortest
        let graph = make_multi_bookmark_segment(&["wip-auth-feature", "tmp-auth"]);
        let segment = &graph.stack.as_ref().unwrap().segments[0];
        let selected = select_bookmark_for_segment(segment, None);
        // Falls back to shortest
        assert_eq!(selected.name, "tmp-auth");
    }

    #[test]
    fn test_select_bookmark_alphabetical_tiebreaker() {
        // Use equal-length names to test alphabetical tiebreaker
        let graph = make_multi_bookmark_segment(&["bbb", "aaa"]);
        let segment = &graph.stack.as_ref().unwrap().segments[0];
        let selected = select_bookmark_for_segment(segment, None);
        // Same length (3 chars each), alphabetically first wins
        assert_eq!(selected.name, "aaa");
    }

    #[test]
    fn test_select_bookmark_single_returns_it() {
        let graph = make_linear_stack(&["solo"]);
        let segment = &graph.stack.as_ref().unwrap().segments[0];
        let selected = select_bookmark_for_segment(segment, None);
        assert_eq!(selected.name, "solo");
    }

    // === Deep stack test ===

    #[test]
    fn test_analyze_10_level_deep_stack() {
        let names: Vec<String> = (0..10).map(|i| format!("feat-{i}")).collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let graph = make_linear_stack(&name_refs);
        let analysis = analyze_submission(&graph, Some("feat-9")).unwrap();

        assert_eq!(analysis.segments.len(), 10);
        assert_eq!(analysis.segments[0].bookmark.name, "feat-0");
        assert_eq!(analysis.segments[9].bookmark.name, "feat-9");
    }
}

mod detection_test {
    use jj_ryu::error::Error;
    use jj_ryu::platform::{detect_platform, parse_repo_info};
    use jj_ryu::types::Platform;

    #[test]
    fn test_github_ssh_without_git_extension() {
        let config = parse_repo_info("git@github.com:owner/repo").unwrap();
        assert_eq!(config.platform, Platform::GitHub);
        assert_eq!(config.owner, "owner");
        assert_eq!(config.repo, "repo");
    }

    #[test]
    fn test_github_https_without_git_extension() {
        let config = parse_repo_info("https://github.com/owner/repo").unwrap();
        assert_eq!(config.platform, Platform::GitHub);
        assert_eq!(config.owner, "owner");
        assert_eq!(config.repo, "repo");
    }

    #[test]
    fn test_gitlab_deeply_nested_groups() {
        let config = parse_repo_info("https://gitlab.com/a/b/c/d/repo.git").unwrap();
        assert_eq!(config.platform, Platform::GitLab);
        assert_eq!(config.owner, "a/b/c/d");
        assert_eq!(config.repo, "repo");
    }

    #[test]
    fn test_gitlab_ssh_nested_groups() {
        let config = parse_repo_info("git@gitlab.com:group/subgroup/repo.git").unwrap();
        assert_eq!(config.platform, Platform::GitLab);
        assert_eq!(config.owner, "group/subgroup");
        assert_eq!(config.repo, "repo");
    }

    // Note: GitHub Enterprise and GitLab self-hosted detection tests
    // are skipped here because they require modifying env vars, which
    // is unsafe in Rust 2024 edition and the project forbids unsafe code.
    // These are tested inline in src/platform/detection.rs

    #[test]
    fn test_unknown_platform_returns_none() {
        let platform = detect_platform("https://bitbucket.org/owner/repo.git");
        assert_eq!(platform, None);
    }

    #[test]
    fn test_parse_unknown_platform_returns_error() {
        let result = parse_repo_info("https://bitbucket.org/owner/repo.git");
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_url_returns_no_supported_remotes() {
        // Invalid URLs that can't be parsed return NoSupportedRemotes
        let result = parse_repo_info("not-a-valid-url");
        match result {
            Err(Error::NoSupportedRemotes) => {} // Expected
            other => panic!("Expected NoSupportedRemotes error, got: {other:?}"),
        }
    }

    #[test]
    fn test_github_url_with_trailing_slash() {
        // Trailing slashes are stripped before parsing
        let config = parse_repo_info("https://github.com/owner/repo/").unwrap();
        assert_eq!(config.platform, Platform::GitHub);
        assert_eq!(config.owner, "owner");
        assert_eq!(config.repo, "repo");
    }

    #[test]
    fn test_github_url_with_multiple_trailing_slashes() {
        let config = parse_repo_info("https://github.com/owner/repo///").unwrap();
        assert_eq!(config.owner, "owner");
        assert_eq!(config.repo, "repo");
    }

    #[test]
    fn test_gitlab_single_level_group() {
        let config = parse_repo_info("https://gitlab.com/owner/repo.git").unwrap();
        assert_eq!(config.platform, Platform::GitLab);
        assert_eq!(config.owner, "owner");
        assert_eq!(config.repo, "repo");
    }

    #[test]
    fn test_github_with_git_extension() {
        let config = parse_repo_info("git@github.com:owner/repo.git").unwrap();
        assert_eq!(config.platform, Platform::GitHub);
        assert_eq!(config.repo, "repo"); // .git should be stripped
    }
}

mod plan_test {
    use crate::common::{MockPlatformService, github_config, make_linear_stack, make_pr};
    use jj_ryu::submit::{ExecutionStep, analyze_submission, create_submission_plan};

    #[tokio::test]
    async fn test_plan_new_stack_no_existing_prs() {
        let graph = make_linear_stack(&["feat-a", "feat-b"]);
        let analysis = analyze_submission(&graph, Some("feat-b")).unwrap();

        // Mock returns None for all find_existing_pr calls (default behavior)
        let mock = MockPlatformService::with_config(github_config());

        let plan = create_submission_plan(&analysis, &mock, "origin", "main")
            .await
            .unwrap();

        assert_eq!(plan.count_creates(), 2);
        assert_eq!(plan.count_updates(), 0);

        // Find CreatePr steps and verify them
        let creates: Vec<_> = plan
            .execution_steps
            .iter()
            .filter_map(|s| match s {
                ExecutionStep::CreatePr(c) => Some(c),
                _ => None,
            })
            .collect();

        // First PR should target main
        assert_eq!(creates[0].bookmark.name, "feat-a");
        assert_eq!(creates[0].base_branch, "main");

        // Second PR should target first bookmark
        assert_eq!(creates[1].bookmark.name, "feat-b");
        assert_eq!(creates[1].base_branch, "feat-a");
    }

    #[tokio::test]
    async fn test_plan_update_existing_pr_base() {
        let graph = make_linear_stack(&["feat-a", "feat-b"]);
        let analysis = analyze_submission(&graph, Some("feat-b")).unwrap();

        let mock = MockPlatformService::with_config(github_config());
        // feat-a: no existing PR (default)
        // feat-b: existing PR with wrong base (main instead of feat-a)
        mock.set_find_pr_response("feat-b", Some(make_pr(123, "feat-b", "main")));

        let plan = create_submission_plan(&analysis, &mock, "origin", "main")
            .await
            .unwrap();

        assert_eq!(plan.count_creates(), 1);
        assert_eq!(plan.count_updates(), 1);

        let update = plan
            .execution_steps
            .iter()
            .find_map(|s| match s {
                ExecutionStep::UpdateBase(u) => Some(u),
                _ => None,
            })
            .expect("should have update step");

        assert_eq!(update.bookmark.name, "feat-b");
        assert_eq!(update.current_base, "main");
        assert_eq!(update.expected_base, "feat-a");
    }

    #[tokio::test]
    async fn test_plan_all_prs_exist_correct_base() {
        let graph = make_linear_stack(&["feat-a", "feat-b"]);
        let analysis = analyze_submission(&graph, Some("feat-b")).unwrap();

        let mock = MockPlatformService::with_config(github_config());
        // Both PRs exist with correct bases
        mock.set_find_pr_response("feat-a", Some(make_pr(1, "feat-a", "main")));
        mock.set_find_pr_response("feat-b", Some(make_pr(2, "feat-b", "feat-a")));

        let plan = create_submission_plan(&analysis, &mock, "origin", "main")
            .await
            .unwrap();

        // Nothing to create or update (only pushes if needed)
        assert_eq!(plan.count_creates(), 0);
        assert_eq!(plan.count_updates(), 0);

        // But we should have existing PRs tracked
        assert_eq!(plan.existing_prs.len(), 2);
    }

    #[tokio::test]
    async fn test_plan_synced_bookmark_not_in_push_list() {
        let mut graph = make_linear_stack(&["feat-a"]);
        // Mark bookmark as synced
        if let Some(bm) = graph.bookmarks.get_mut("feat-a") {
            bm.has_remote = true;
            bm.is_synced = true;
        }
        // Also update in stacks
        if let Some(segment) = graph.stack.as_mut().and_then(|s| s.segments.get_mut(0))
            && let Some(bm) = segment.bookmarks.get_mut(0)
        {
            bm.has_remote = true;
            bm.is_synced = true;
        }

        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();
        let mock = MockPlatformService::with_config(github_config());

        let plan = create_submission_plan(&analysis, &mock, "origin", "main")
            .await
            .unwrap();

        // Synced bookmark should not be in push list
        assert_eq!(plan.count_pushes(), 0);
    }

    #[tokio::test]
    async fn test_plan_unsynced_bookmark_in_push_list() {
        let graph = make_linear_stack(&["feat-a"]);
        // Default bookmarks from fixtures are not synced
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();
        let mock = MockPlatformService::with_config(github_config());

        let plan = create_submission_plan(&analysis, &mock, "origin", "main")
            .await
            .unwrap();

        assert_eq!(plan.count_pushes(), 1);

        let push = plan
            .execution_steps
            .iter()
            .find_map(|s| match s {
                ExecutionStep::Push(b) => Some(b),
                _ => None,
            })
            .expect("should have push step");

        assert_eq!(push.name, "feat-a");
    }

    // === Mock verification tests ===

    #[tokio::test]
    async fn test_plan_queries_platform_for_each_bookmark() {
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();
        let mock = MockPlatformService::with_config(github_config());

        let _ = create_submission_plan(&analysis, &mock, "origin", "main")
            .await
            .unwrap();

        // Verify find_existing_pr was called for each bookmark
        mock.assert_find_pr_called_for(&["feat-a", "feat-b", "feat-c"]);
    }

    #[tokio::test]
    async fn test_plan_has_remote_true_but_not_synced_needs_push() {
        let mut graph = make_linear_stack(&["feat-a"]);
        // has_remote=true but is_synced=false (e.g., local changes after push)
        if let Some(bm) = graph.bookmarks.get_mut("feat-a") {
            bm.has_remote = true;
            bm.is_synced = false;
        }
        if let Some(segment) = graph.stack.as_mut().and_then(|s| s.segments.get_mut(0))
            && let Some(bm) = segment.bookmarks.get_mut(0)
        {
            bm.has_remote = true;
            bm.is_synced = false;
        }

        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();
        let mock = MockPlatformService::with_config(github_config());

        let plan = create_submission_plan(&analysis, &mock, "origin", "main")
            .await
            .unwrap();

        // Should still need push because is_synced=false
        assert_eq!(plan.count_pushes(), 1);
    }

    #[tokio::test]
    async fn test_plan_multiple_base_updates_needed() {
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        let mock = MockPlatformService::with_config(github_config());
        // All PRs exist but with wrong bases (all pointing to main)
        mock.set_find_pr_response("feat-a", Some(make_pr(1, "feat-a", "main")));
        mock.set_find_pr_response("feat-b", Some(make_pr(2, "feat-b", "main"))); // Should be feat-a
        mock.set_find_pr_response("feat-c", Some(make_pr(3, "feat-c", "main"))); // Should be feat-b

        let plan = create_submission_plan(&analysis, &mock, "origin", "main")
            .await
            .unwrap();

        // feat-a is correct (base=main), feat-b and feat-c need updates
        assert_eq!(plan.count_creates(), 0);
        assert_eq!(plan.count_updates(), 2);

        let updates: Vec<_> = plan
            .execution_steps
            .iter()
            .filter_map(|s| match s {
                ExecutionStep::UpdateBase(u) => Some(u),
                _ => None,
            })
            .collect();

        assert_eq!(updates[0].bookmark.name, "feat-b");
        assert_eq!(updates[0].expected_base, "feat-a");
        assert_eq!(updates[1].bookmark.name, "feat-c");
        assert_eq!(updates[1].expected_base, "feat-b");
    }

    // === Error handling tests ===

    #[tokio::test]
    async fn test_plan_handles_find_pr_error() {
        let graph = make_linear_stack(&["feat-a", "feat-b"]);
        let analysis = analyze_submission(&graph, Some("feat-b")).unwrap();

        let mock = MockPlatformService::with_config(github_config());
        mock.fail_find_pr("rate limited");

        let result = create_submission_plan(&analysis, &mock, "origin", "main").await;

        assert!(result.is_err(), "Expected error when find_pr fails");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("rate limited"),
            "Error should contain original message: {err}"
        );
    }

    #[tokio::test]
    async fn test_plan_error_is_platform_type() {
        use jj_ryu::error::Error;

        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mock = MockPlatformService::with_config(github_config());
        mock.fail_find_pr("API unavailable");

        let result = create_submission_plan(&analysis, &mock, "origin", "main").await;

        match result {
            Err(Error::Platform(msg)) => {
                assert_eq!(msg, "API unavailable");
            }
            other => panic!("Expected Platform error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_plan_fails_fast_on_first_error() {
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        let mock = MockPlatformService::with_config(github_config());
        mock.fail_find_pr("connection failed");

        let result = create_submission_plan(&analysis, &mock, "origin", "main").await;

        assert!(result.is_err());
        // Should have attempted at least one call before failing
        let calls = mock.get_find_pr_calls();
        assert!(!calls.is_empty(), "Should have made at least one API call");
        // But should not have completed all calls (fail fast)
        assert!(
            calls.len() <= 3,
            "Should fail fast, not retry all bookmarks"
        );
    }
}

mod stack_comment_test {
    use jj_ryu::submit::{
        COMMENT_DATA_PREFIX, STACK_COMMENT_THIS_PR, StackCommentData, StackItem, SubmissionPlan,
        build_stack_comment_data, format_stack_comment,
    };
    use jj_ryu::types::{Bookmark, NarrowedBookmarkSegment, PullRequest};
    use std::collections::HashMap;

    fn make_bookmark(name: &str) -> Bookmark {
        Bookmark {
            name: name.to_string(),
            commit_id: format!("{name}_commit"),
            change_id: format!("{name}_change"),
            has_remote: false,
            is_synced: false,
        }
    }

    fn make_pr(number: u64, bookmark: &str) -> PullRequest {
        PullRequest {
            number,
            html_url: format!("https://github.com/test/test/pull/{number}"),
            base_ref: "main".to_string(),
            head_ref: bookmark.to_string(),
            title: format!("PR for {bookmark}"),
            node_id: Some(format!("PR_node_{number}")),
            is_draft: false,
        }
    }

    fn make_stack_item(name: &str, number: u64) -> StackItem {
        StackItem {
            bookmark_name: name.to_string(),
            pr_url: format!("https://github.com/test/test/pull/{number}"),
            pr_number: number,
            pr_title: format!("feat: {name}"),
        }
    }

    #[test]
    fn test_build_stack_comment_data_single_pr() {
        let plan = SubmissionPlan {
            segments: vec![NarrowedBookmarkSegment {
                bookmark: make_bookmark("feat-a"),
                changes: vec![],
            }],
            constraints: vec![],
            execution_steps: vec![],
            existing_prs: HashMap::new(),
            remote: "origin".to_string(),
            default_branch: "main".to_string(),
        };

        let mut bookmark_to_pr = HashMap::new();
        bookmark_to_pr.insert("feat-a".to_string(), make_pr(1, "feat-a"));

        let data = build_stack_comment_data(&plan, &bookmark_to_pr);

        assert_eq!(data.version, 1);
        assert_eq!(data.base_branch, "main");
        assert_eq!(data.stack.len(), 1);
        assert_eq!(data.stack[0].bookmark_name, "feat-a");
        assert_eq!(data.stack[0].pr_number, 1);
    }

    #[test]
    fn test_build_stack_comment_data_three_pr_stack() {
        let plan = SubmissionPlan {
            segments: vec![
                NarrowedBookmarkSegment {
                    bookmark: make_bookmark("feat-a"),
                    changes: vec![],
                },
                NarrowedBookmarkSegment {
                    bookmark: make_bookmark("feat-b"),
                    changes: vec![],
                },
                NarrowedBookmarkSegment {
                    bookmark: make_bookmark("feat-c"),
                    changes: vec![],
                },
            ],
            constraints: vec![],
            execution_steps: vec![],
            existing_prs: HashMap::new(),
            remote: "origin".to_string(),
            default_branch: "main".to_string(),
        };

        let mut bookmark_to_pr = HashMap::new();
        bookmark_to_pr.insert("feat-a".to_string(), make_pr(1, "feat-a"));
        bookmark_to_pr.insert("feat-b".to_string(), make_pr(2, "feat-b"));
        bookmark_to_pr.insert("feat-c".to_string(), make_pr(3, "feat-c"));

        let data = build_stack_comment_data(&plan, &bookmark_to_pr);

        assert_eq!(data.stack.len(), 3);
        assert_eq!(data.stack[0].pr_number, 1);
        assert_eq!(data.stack[1].pr_number, 2);
        assert_eq!(data.stack[2].pr_number, 3);
    }

    #[test]
    fn test_format_body_marks_current_pr() {
        let data = StackCommentData {
            version: 1,
            stack: vec![make_stack_item("feat-a", 1), make_stack_item("feat-b", 2)],
            base_branch: "main".to_string(),
        };

        // Format for second PR (index 1)
        let body = format_stack_comment(&data, 1).unwrap();

        // PR #2 should have the marker
        assert!(
            body.contains(&format!("#{} {STACK_COMMENT_THIS_PR}", 2)),
            "body should mark PR #2 as current: {body}"
        );

        // PR #1 should NOT have the marker
        assert!(
            !body.contains(&format!("#{} {STACK_COMMENT_THIS_PR}", 1)),
            "body should NOT mark PR #1 as current: {body}"
        );
    }

    #[test]
    fn test_format_body_reverse_order() {
        let data = StackCommentData {
            version: 1,
            stack: vec![
                make_stack_item("feat-a", 1),
                make_stack_item("feat-b", 2),
                make_stack_item("feat-c", 3),
            ],
            base_branch: "main".to_string(),
        };

        let body = format_stack_comment(&data, 0).unwrap();

        // Find positions of each PR in the body
        let pos_1 = body.find("#1").expect("should contain #1");
        let pos_2 = body.find("#2").expect("should contain #2");
        let pos_3 = body.find("#3").expect("should contain #3");

        // Reverse order means #3 (leaf) comes first, #1 (root) comes last
        assert!(pos_3 < pos_2, "PR #3 should appear before #2");
        assert!(pos_2 < pos_1, "PR #2 should appear before #1");
    }

    #[test]
    fn test_format_body_contains_marker() {
        let data = StackCommentData {
            version: 1,
            stack: vec![make_stack_item("feat-a", 1)],
            base_branch: "main".to_string(),
        };

        let body = format_stack_comment(&data, 0).unwrap();

        assert!(
            body.contains(COMMENT_DATA_PREFIX),
            "body should contain data prefix"
        );
    }

    #[test]
    fn test_format_body_contains_base_branch() {
        let data = StackCommentData {
            version: 1,
            stack: vec![make_stack_item("feat-a", 1)],
            base_branch: "develop".to_string(),
        };

        let body = format_stack_comment(&data, 0).unwrap();

        assert!(
            body.contains("`develop`"),
            "body should contain base branch: {body}"
        );
    }

    #[test]
    fn test_format_body_contains_pr_title() {
        let data = StackCommentData {
            version: 1,
            stack: vec![make_stack_item("feat-a", 1)],
            base_branch: "main".to_string(),
        };

        let body = format_stack_comment(&data, 0).unwrap();

        assert!(
            body.contains("feat: feat-a"),
            "body should contain PR title: {body}"
        );
    }
}

mod sync_test {
    use jj_ryu::error::Error;
    use jj_ryu::repo::select_remote;
    use jj_ryu::types::GitRemote;

    fn make_remote(name: &str) -> GitRemote {
        GitRemote {
            name: name.to_string(),
            url: format!("https://github.com/test/{name}.git"),
        }
    }

    #[test]
    fn test_select_remote_single_remote() {
        let remotes = vec![make_remote("upstream")];
        let result = select_remote(&remotes, None).unwrap();
        assert_eq!(result, "upstream");
    }

    #[test]
    fn test_select_remote_prefers_origin() {
        let remotes = vec![
            make_remote("upstream"),
            make_remote("origin"),
            make_remote("fork"),
        ];
        let result = select_remote(&remotes, None).unwrap();
        assert_eq!(result, "origin");
    }

    #[test]
    fn test_select_remote_no_origin_uses_first() {
        let remotes = vec![make_remote("upstream"), make_remote("fork")];
        let result = select_remote(&remotes, None).unwrap();
        assert_eq!(result, "upstream");
    }

    #[test]
    fn test_select_remote_specified_exists() {
        let remotes = vec![make_remote("origin"), make_remote("fork")];
        let result = select_remote(&remotes, Some("fork")).unwrap();
        assert_eq!(result, "fork");
    }

    #[test]
    fn test_select_remote_specified_not_found() {
        let remotes = vec![make_remote("origin")];
        let result = select_remote(&remotes, Some("nonexistent"));
        match result {
            Err(Error::RemoteNotFound(name)) => assert_eq!(name, "nonexistent"),
            other => panic!("Expected RemoteNotFound error, got: {other:?}"),
        }
    }

    #[test]
    fn test_select_remote_none_available() {
        let remotes: Vec<GitRemote> = vec![];
        let result = select_remote(&remotes, None);
        match result {
            Err(Error::NoSupportedRemotes) => {}
            other => panic!("Expected NoSupportedRemotes error, got: {other:?}"),
        }
    }
}

mod merge_plan_test {
    use crate::common::make_linear_stack;
    use jj_ryu::merge::{create_merge_plan, MergeConfidence, MergePlanOptions, MergeStep, PrInfo};
    use jj_ryu::submit::analyze_submission;
    use jj_ryu::types::{MergeMethod, MergeReadiness, PrState, PullRequestDetails};
    use std::collections::HashMap;

    /// Helper to create a mergeable PrInfo with base_ref set to "main".
    ///
    /// NOTE: This creates a "flat" PR where all PRs target main directly.
    /// For realistic stacked PR scenarios where PRs target their parent's branch,
    /// use `make_mergeable_pr_info_with_base` instead.
    fn make_mergeable_pr_info(bookmark: &str, pr_number: u64, title: &str) -> PrInfo {
        PrInfo {
            bookmark: bookmark.to_string(),
            details: PullRequestDetails {
                number: pr_number,
                title: title.to_string(),
                body: Some(format!("PR body for {bookmark}")),
                state: PrState::Open,
                is_draft: false,
                mergeable: Some(true),
                head_ref: bookmark.to_string(),
                base_ref: "main".to_string(),
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
            },
            readiness: MergeReadiness {
                is_approved: true,
                ci_passed: true,
                is_mergeable: Some(true),
                is_draft: false,
                blocking_reasons: vec![],
                uncertainties: vec![],
            },
        }
    }

    /// Helper to create a blocked PrInfo
    fn make_blocked_pr_info(
        bookmark: &str,
        pr_number: u64,
        title: &str,
        reasons: Vec<String>,
    ) -> PrInfo {
        PrInfo {
            bookmark: bookmark.to_string(),
            details: PullRequestDetails {
                number: pr_number,
                title: title.to_string(),
                body: Some(format!("PR body for {bookmark}")),
                state: PrState::Open,
                is_draft: false,
                mergeable: Some(true),
                head_ref: bookmark.to_string(),
                base_ref: "main".to_string(),
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
            },
            readiness: MergeReadiness {
                is_approved: false,
                ci_passed: true,
                is_mergeable: Some(true),
                is_draft: false,
                blocking_reasons: reasons,
                uncertainties: vec![],
            },
        }
    }

    /// Helper to create a PrInfo with uncertain merge status (GitHub still computing)
    fn make_uncertain_pr_info(bookmark: &str, pr_number: u64, title: &str) -> PrInfo {
        PrInfo {
            bookmark: bookmark.to_string(),
            details: PullRequestDetails {
                number: pr_number,
                title: title.to_string(),
                body: Some(format!("PR body for {bookmark}")),
                state: PrState::Open,
                is_draft: false,
                mergeable: None, // Unknown - GitHub still computing
                head_ref: bookmark.to_string(),
                base_ref: "main".to_string(),
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
            },
            readiness: MergeReadiness {
                is_approved: true,
                ci_passed: true,
                is_mergeable: None, // Must match details.mergeable
                is_draft: false,
                blocking_reasons: vec![],
                uncertainties: vec!["Merge status unknown (GitHub still computing)".to_string()],
            },
        }
    }

    #[test]
    fn test_create_merge_plan_single_mergeable() {
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_mergeable_pr_info("feat-a", 1, "Add feature A"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert!(!plan.is_empty());
        assert_eq!(plan.merge_count(), 1);
        assert!(plan.has_actionable);
        assert_eq!(plan.bookmarks_to_clear, vec!["feat-a"]);
        assert!(plan.rebase_target.is_none()); // Nothing left to rebase

        // Verify the step details
        match &plan.steps[0] {
            MergeStep::Merge {
                bookmark,
                pr_number,
                pr_title,
                method,
                confidence,
            } => {
                assert_eq!(bookmark, "feat-a");
                assert_eq!(*pr_number, 1);
                assert_eq!(pr_title, "Add feature A");
                assert_eq!(*method, MergeMethod::Squash);
                assert_eq!(*confidence, MergeConfidence::Certain);
            }
            MergeStep::Skip { .. } => panic!("Expected Merge step, got Skip"),
            MergeStep::RetargetBase { .. } => panic!("Expected Merge step, got RetargetBase"),
        }
    }

    #[test]
    fn test_create_merge_plan_multiple_consecutive_mergeable() {
        // Test with realistic stacked PR base refs:
        // PR1 targets main, PR2 targets feat-a, PR3 targets feat-b
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        let mut pr_info = HashMap::new();
        // PR1 targets main (correct for first PR in stack)
        pr_info.insert(
            "feat-a".to_string(),
            make_mergeable_pr_info_with_base("feat-a", 1, "Add feature A", "main"),
        );
        // PR2 targets feat-a (will need retarget after PR1 merges)
        pr_info.insert(
            "feat-b".to_string(),
            make_mergeable_pr_info_with_base("feat-b", 2, "Add feature B", "feat-a"),
        );
        // PR3 targets feat-b (will need retarget after PR2 merges)
        pr_info.insert(
            "feat-c".to_string(),
            make_mergeable_pr_info_with_base("feat-c", 3, "Add feature C", "feat-b"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert_eq!(plan.merge_count(), 3);
        assert!(plan.has_actionable);
        assert_eq!(
            plan.bookmarks_to_clear,
            vec!["feat-a", "feat-b", "feat-c"]
        );
        assert!(plan.rebase_target.is_none());

        // Should have 5 steps: Merge, Retarget, Merge, Retarget, Merge
        assert_eq!(plan.steps.len(), 5);
        assert!(matches!(&plan.steps[0], MergeStep::Merge { pr_number: 1, .. }));
        assert!(matches!(&plan.steps[1], MergeStep::RetargetBase { pr_number: 2, new_base, .. } if new_base == "main"));
        assert!(matches!(&plan.steps[2], MergeStep::Merge { pr_number: 2, .. }));
        assert!(matches!(&plan.steps[3], MergeStep::RetargetBase { pr_number: 3, new_base, .. } if new_base == "main"));
        assert!(matches!(&plan.steps[4], MergeStep::Merge { pr_number: 3, .. }));
    }

    #[test]
    fn test_create_merge_plan_blocked_pr_stops_chain() {
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_mergeable_pr_info("feat-a", 1, "Add feature A"),
        );
        // feat-b is blocked
        pr_info.insert(
            "feat-b".to_string(),
            make_blocked_pr_info("feat-b", 2, "Add feature B", vec!["Not approved".to_string()]),
        );
        pr_info.insert(
            "feat-c".to_string(),
            make_mergeable_pr_info("feat-c", 3, "Add feature C"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        // Only feat-a should be merged, feat-b is skipped
        assert_eq!(plan.merge_count(), 1);
        assert!(plan.has_actionable);
        assert_eq!(plan.bookmarks_to_clear, vec!["feat-a"]);
        assert_eq!(plan.rebase_target, Some("feat-b".to_string()));

        // Verify steps
        assert_eq!(plan.steps.len(), 2);
        assert!(matches!(&plan.steps[0], MergeStep::Merge { bookmark, .. } if bookmark == "feat-a"));
        assert!(
            matches!(&plan.steps[1], MergeStep::Skip { bookmark, reasons, .. } if bookmark == "feat-b" && reasons.contains(&"Not approved".to_string()))
        );
    }

    #[test]
    fn test_create_merge_plan_first_pr_blocked() {
        let graph = make_linear_stack(&["feat-a", "feat-b"]);
        let analysis = analyze_submission(&graph, Some("feat-b")).unwrap();

        let mut pr_info = HashMap::new();
        // feat-a is blocked - nothing can merge
        pr_info.insert(
            "feat-a".to_string(),
            make_blocked_pr_info(
                "feat-a",
                1,
                "Add feature A",
                vec!["CI not passing".to_string()],
            ),
        );
        pr_info.insert(
            "feat-b".to_string(),
            make_mergeable_pr_info("feat-b", 2, "Add feature B"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert!(plan.is_empty());
        assert_eq!(plan.merge_count(), 0);
        assert!(!plan.has_actionable);
        assert!(plan.bookmarks_to_clear.is_empty());
        assert_eq!(plan.rebase_target, Some("feat-a".to_string()));

        // Should have one Skip step
        assert_eq!(plan.steps.len(), 1);
        assert!(matches!(&plan.steps[0], MergeStep::Skip { bookmark, .. } if bookmark == "feat-a"));
    }

    #[test]
    fn test_create_merge_plan_with_target_bookmark() {
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_mergeable_pr_info("feat-a", 1, "Add feature A"),
        );
        pr_info.insert(
            "feat-b".to_string(),
            make_mergeable_pr_info("feat-b", 2, "Add feature B"),
        );
        pr_info.insert(
            "feat-c".to_string(),
            make_mergeable_pr_info("feat-c", 3, "Add feature C"),
        );

        // Only merge up to feat-b
        let options = MergePlanOptions {
            target_bookmark: Some("feat-b".to_string()),
        };
        let plan = create_merge_plan(&analysis, &pr_info, &options, "main");

        // Should merge feat-a and feat-b, but not feat-c
        assert_eq!(plan.merge_count(), 2);
        assert_eq!(plan.bookmarks_to_clear, vec!["feat-a", "feat-b"]);
        assert_eq!(plan.rebase_target, Some("feat-c".to_string()));
    }

    #[test]
    fn test_create_merge_plan_empty_when_no_prs() {
        let graph = make_linear_stack(&["feat-a", "feat-b"]);
        let analysis = analyze_submission(&graph, Some("feat-b")).unwrap();

        // No PR info provided
        let pr_info: HashMap<String, PrInfo> = HashMap::new();

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert!(plan.is_empty());
        assert_eq!(plan.merge_count(), 0);
        assert!(!plan.has_actionable);
        assert!(plan.steps.is_empty());
    }

    #[test]
    fn test_create_merge_plan_partial_pr_info() {
        // Only some bookmarks have PRs
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        let mut pr_info = HashMap::new();
        // Only feat-b has a PR
        pr_info.insert(
            "feat-b".to_string(),
            make_mergeable_pr_info("feat-b", 2, "Add feature B"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        // feat-a has no PR, so it's skipped. feat-b can merge.
        // But wait - the plan processes in stack order and skips bookmarks without PRs
        assert_eq!(plan.merge_count(), 1);
        assert_eq!(plan.bookmarks_to_clear, vec!["feat-b"]);
    }

    #[test]
    fn test_create_merge_plan_draft_pr_blocks() {
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mut pr_info = HashMap::new();
        let mut info = make_mergeable_pr_info("feat-a", 1, "Add feature A");
        // Make it a draft
        info.details.is_draft = true;
        info.readiness.is_draft = true;
        info.readiness.blocking_reasons = vec!["PR is a draft".to_string()];
        pr_info.insert("feat-a".to_string(), info);

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert!(plan.is_empty());
        assert!(!plan.has_actionable);
        assert!(matches!(&plan.steps[0], MergeStep::Skip { reasons, .. } if reasons.contains(&"PR is a draft".to_string())));
    }

    #[test]
    fn test_create_merge_plan_not_approved_blocks() {
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_blocked_pr_info("feat-a", 1, "Add feature A", vec!["Not approved".to_string()]),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert!(plan.is_empty());
        assert!(matches!(&plan.steps[0], MergeStep::Skip { reasons, .. } if reasons.contains(&"Not approved".to_string())));
    }

    #[test]
    fn test_create_merge_plan_ci_failing_blocks() {
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mut pr_info = HashMap::new();
        let mut info = make_mergeable_pr_info("feat-a", 1, "Add feature A");
        info.readiness.ci_passed = false;
        info.readiness.blocking_reasons = vec!["CI not passing".to_string()];
        pr_info.insert("feat-a".to_string(), info);

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert!(plan.is_empty());
        assert!(matches!(&plan.steps[0], MergeStep::Skip { reasons, .. } if reasons.contains(&"CI not passing".to_string())));
    }

    #[test]
    fn test_create_merge_plan_merge_conflicts_blocks() {
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mut pr_info = HashMap::new();
        let mut info = make_mergeable_pr_info("feat-a", 1, "Add feature A");
        info.readiness.is_mergeable = Some(false);
        info.readiness.blocking_reasons = vec!["Has merge conflicts".to_string()];
        pr_info.insert("feat-a".to_string(), info);

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert!(plan.is_empty());
        assert!(matches!(&plan.steps[0], MergeStep::Skip { reasons, .. } if reasons.contains(&"Has merge conflicts".to_string())));
    }

    #[test]
    fn test_merge_plan_is_empty_with_only_skips() {
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_blocked_pr_info("feat-a", 1, "Add feature A", vec!["Not approved".to_string()]),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        // is_empty() should return true when there are only Skip steps
        assert!(plan.is_empty());
        // But steps is not empty - it contains Skip steps
        assert!(!plan.steps.is_empty());
    }

    #[test]
    fn test_merge_plan_merge_count() {
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_mergeable_pr_info("feat-a", 1, "Add feature A"),
        );
        pr_info.insert(
            "feat-b".to_string(),
            make_blocked_pr_info("feat-b", 2, "Add feature B", vec!["Not approved".to_string()]),
        );
        pr_info.insert(
            "feat-c".to_string(),
            make_mergeable_pr_info("feat-c", 3, "Add feature C"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        // merge_count should only count Merge steps, not Skip steps
        assert_eq!(plan.merge_count(), 1);
        assert_eq!(plan.steps.len(), 2); // 1 Merge + 1 Skip
    }

    #[test]
    fn test_create_merge_plan_uncertain_mergeable_has_uncertain_confidence() {
        // PR with is_mergeable: None should produce Merge with Uncertain confidence
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_uncertain_pr_info("feat-a", 1, "Feature A"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert!(!plan.is_empty());
        assert!(plan.has_actionable);
        match &plan.steps[0] {
            MergeStep::Merge { confidence, .. } => {
                assert!(matches!(confidence, MergeConfidence::Uncertain(_)));
                if let MergeConfidence::Uncertain(reason) = confidence {
                    assert!(reason.contains("Merge status unknown"));
                }
            }
            _ => panic!("Expected Merge step"),
        }
    }

    #[test]
    fn test_blocked_with_unknown_mergeable_still_skips() {
        // If not approved AND mergeable unknown, should Skip (blocker takes precedence)
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mut pr_info = HashMap::new();
        let mut info = make_uncertain_pr_info("feat-a", 1, "Feature A");
        info.readiness.is_approved = false;
        info.readiness.blocking_reasons = vec!["Not approved".to_string()];
        pr_info.insert("feat-a".to_string(), info);

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        assert!(plan.is_empty()); // No Merge steps
        assert!(
            matches!(&plan.steps[0], MergeStep::Skip { reasons, .. } if reasons.contains(&"Not approved".to_string()))
        );
    }

    #[test]
    fn test_merge_readiness_is_blocked() {
        // Unit tests for is_blocked() with various combinations
        let base = MergeReadiness {
            is_approved: true,
            ci_passed: true,
            is_mergeable: Some(true),
            is_draft: false,
            blocking_reasons: vec![],
            uncertainties: vec![],
        };
        assert!(!base.is_blocked());

        // Not approved blocks
        let mut r = base.clone();
        r.is_approved = false;
        assert!(r.is_blocked());

        // CI failing blocks
        let mut r = base.clone();
        r.ci_passed = false;
        assert!(r.is_blocked());

        // Conflicts block
        let mut r = base.clone();
        r.is_mergeable = Some(false);
        assert!(r.is_blocked());

        // Unknown does NOT block
        let mut r = base.clone();
        r.is_mergeable = None;
        assert!(!r.is_blocked());

        // Draft blocks
        let mut r = base;
        r.is_draft = true;
        assert!(r.is_blocked());
    }

    #[test]
    fn test_merge_readiness_uncertainty() {
        // Unit tests for uncertainty() method
        let mut r = MergeReadiness {
            is_approved: true,
            ci_passed: true,
            is_mergeable: None,
            is_draft: false,
            blocking_reasons: vec![],
            uncertainties: vec![],
        };
        assert!(r.uncertainty().is_none());

        r.uncertainties = vec!["Reason 1".to_string()];
        assert_eq!(r.uncertainty(), Some("Reason 1"));

        r.uncertainties = vec!["Reason 1".to_string(), "Reason 2".to_string()];
        assert_eq!(r.uncertainty(), Some("Reason 1")); // Returns first only
    }

    // =========================================================================
    // Retarget step generation tests
    // =========================================================================

    /// Helper to create a PrInfo with a specific base_ref (for retarget testing)
    fn make_mergeable_pr_info_with_base(
        bookmark: &str,
        pr_number: u64,
        title: &str,
        base_ref: &str,
    ) -> PrInfo {
        PrInfo {
            bookmark: bookmark.to_string(),
            details: PullRequestDetails {
                number: pr_number,
                title: title.to_string(),
                body: Some(format!("PR body for {bookmark}")),
                state: PrState::Open,
                is_draft: false,
                mergeable: Some(true),
                head_ref: bookmark.to_string(),
                base_ref: base_ref.to_string(),
                html_url: format!("https://github.com/test/repo/pull/{pr_number}"),
            },
            readiness: MergeReadiness {
                is_approved: true,
                ci_passed: true,
                is_mergeable: Some(true),
                is_draft: false,
                blocking_reasons: vec![],
                uncertainties: vec![],
            },
        }
    }

    #[test]
    fn test_create_merge_plan_generates_retarget_steps() {
        // 3-PR stack, all mergeable
        // Expected: Merge(1), Retarget(2), Merge(2), Retarget(3), Merge(3)
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        let mut pr_info = HashMap::new();
        // PR1 targets main (correct)
        pr_info.insert(
            "feat-a".to_string(),
            make_mergeable_pr_info_with_base("feat-a", 1, "Add feature A", "main"),
        );
        // PR2 targets feat-a (will need retarget after PR1 merges)
        pr_info.insert(
            "feat-b".to_string(),
            make_mergeable_pr_info_with_base("feat-b", 2, "Add feature B", "feat-a"),
        );
        // PR3 targets feat-b (will need retarget after PR2 merges)
        pr_info.insert(
            "feat-c".to_string(),
            make_mergeable_pr_info_with_base("feat-c", 3, "Add feature C", "feat-b"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        // Should have 5 steps: Merge, Retarget, Merge, Retarget, Merge
        assert_eq!(plan.steps.len(), 5);
        assert_eq!(plan.merge_count(), 3);

        // Step 0: Merge PR1
        assert!(matches!(&plan.steps[0], MergeStep::Merge { pr_number: 1, .. }));

        // Step 1: Retarget PR2 from feat-a to main
        match &plan.steps[1] {
            MergeStep::RetargetBase {
                pr_number,
                old_base,
                new_base,
                ..
            } => {
                assert_eq!(*pr_number, 2);
                assert_eq!(old_base, "feat-a");
                assert_eq!(new_base, "main");
            }
            _ => panic!("Expected RetargetBase step at index 1"),
        }

        // Step 2: Merge PR2
        assert!(matches!(&plan.steps[2], MergeStep::Merge { pr_number: 2, .. }));

        // Step 3: Retarget PR3 from feat-b to main
        match &plan.steps[3] {
            MergeStep::RetargetBase {
                pr_number,
                old_base,
                new_base,
                ..
            } => {
                assert_eq!(*pr_number, 3);
                assert_eq!(old_base, "feat-b");
                assert_eq!(new_base, "main");
            }
            _ => panic!("Expected RetargetBase step at index 3"),
        }

        // Step 4: Merge PR3
        assert!(matches!(&plan.steps[4], MergeStep::Merge { pr_number: 3, .. }));

        // Verify trunk_branch is set
        assert_eq!(plan.trunk_branch, "main");
    }

    #[test]
    fn test_create_merge_plan_no_retarget_after_skip() {
        // 3-PR stack, PR2 blocked
        // Expected: Merge(1), Skip(2) - no retarget because we stop at skip
        let graph = make_linear_stack(&["feat-a", "feat-b", "feat-c"]);
        let analysis = analyze_submission(&graph, Some("feat-c")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_mergeable_pr_info_with_base("feat-a", 1, "Add feature A", "main"),
        );
        // PR2 is blocked
        pr_info.insert(
            "feat-b".to_string(),
            make_blocked_pr_info("feat-b", 2, "Add feature B", vec!["Not approved".to_string()]),
        );
        pr_info.insert(
            "feat-c".to_string(),
            make_mergeable_pr_info_with_base("feat-c", 3, "Add feature C", "feat-b"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        // Should have 2 steps: Merge(1), Skip(2) - no retarget steps
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.merge_count(), 1);

        assert!(matches!(&plan.steps[0], MergeStep::Merge { pr_number: 1, .. }));
        assert!(matches!(&plan.steps[1], MergeStep::Skip { pr_number: 2, .. }));
    }

    #[test]
    fn test_create_merge_plan_single_pr_no_retarget() {
        // 1-PR stack - nothing to retarget after last merge
        let graph = make_linear_stack(&["feat-a"]);
        let analysis = analyze_submission(&graph, Some("feat-a")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_mergeable_pr_info_with_base("feat-a", 1, "Add feature A", "main"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        // Should have 1 step: Merge only, no retarget
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.merge_count(), 1);
        assert!(matches!(&plan.steps[0], MergeStep::Merge { pr_number: 1, .. }));
    }

    #[test]
    fn test_create_merge_plan_skips_redundant_retarget() {
        // If PR2 already targets main, no retarget needed
        let graph = make_linear_stack(&["feat-a", "feat-b"]);
        let analysis = analyze_submission(&graph, Some("feat-b")).unwrap();

        let mut pr_info = HashMap::new();
        pr_info.insert(
            "feat-a".to_string(),
            make_mergeable_pr_info_with_base("feat-a", 1, "Add feature A", "main"),
        );
        // PR2 already targets main (unusual but possible)
        pr_info.insert(
            "feat-b".to_string(),
            make_mergeable_pr_info_with_base("feat-b", 2, "Add feature B", "main"),
        );

        let plan = create_merge_plan(&analysis, &pr_info, &MergePlanOptions::default(), "main");

        // Should have 2 steps: Merge, Merge - no retarget because base is already main
        assert_eq!(plan.steps.len(), 2);
        assert!(matches!(&plan.steps[0], MergeStep::Merge { pr_number: 1, .. }));
        assert!(matches!(&plan.steps[1], MergeStep::Merge { pr_number: 2, .. }));
    }
}

mod merge_execution_test {
    use crate::common::{github_config, MockPlatformService};
    use jj_ryu::merge::{execute_merge, MergeConfidence, MergePlan, MergeStep};
    use jj_ryu::submit::NoopProgress;
    use jj_ryu::types::{MergeMethod, MergeResult};

    #[tokio::test]
    async fn test_merge_uncertain_pr_succeeds() {
        // Setup: PR with uncertain merge status that will succeed
        let mock = MockPlatformService::with_config(github_config());
        mock.setup_uncertain_pr(1, "feat-a", "Feature A");

        // Create a simple plan with one uncertain merge
        let plan = MergePlan {
            steps: vec![MergeStep::Merge {
                bookmark: "feat-a".to_string(),
                pr_number: 1,
                pr_title: "Feature A".to_string(),
                method: MergeMethod::Squash,
                confidence: MergeConfidence::Uncertain(
                    "Merge status unknown (GitHub still computing)".to_string(),
                ),
            }],
            bookmarks_to_clear: vec!["feat-a".to_string()],
            rebase_target: None,
            has_actionable: true,
            trunk_branch: "main".to_string(),
        };

        let progress = NoopProgress;
        let result = execute_merge(&plan, &mock, &progress).await.unwrap();

        // Verify: merge succeeded despite uncertainty
        assert!(result.is_success());
        assert_eq!(result.merged_bookmarks, vec!["feat-a"]);
        assert!(!result.was_uncertain); // Only set on failure
    }

    #[tokio::test]
    async fn test_merge_uncertain_pr_fails_sets_was_uncertain() {
        let mock = MockPlatformService::with_config(github_config());
        // Setup PR that will fail to merge
        mock.setup_uncertain_pr(1, "feat-a", "Feature A");
        mock.set_merge_response(
            1,
            MergeResult {
                merged: false,
                sha: None,
                message: Some("Merge conflict".to_string()),
            },
        );

        let plan = MergePlan {
            steps: vec![MergeStep::Merge {
                bookmark: "feat-a".to_string(),
                pr_number: 1,
                pr_title: "Feature A".to_string(),
                method: MergeMethod::Squash,
                confidence: MergeConfidence::Uncertain(
                    "Merge status unknown".to_string(),
                ),
            }],
            bookmarks_to_clear: vec!["feat-a".to_string()],
            rebase_target: None,
            has_actionable: true,
            trunk_branch: "main".to_string(),
        };

        let progress = NoopProgress;
        let result = execute_merge(&plan, &mock, &progress).await.unwrap();

        // Verify: merge failed and was_uncertain is set
        assert!(!result.is_success());
        assert!(result.was_uncertain); // Key assertion
        assert_eq!(result.failed_bookmark, Some("feat-a".to_string()));
        assert_eq!(result.error_message, Some("Merge conflict".to_string()));
    }

    #[tokio::test]
    async fn test_merge_certain_pr_fails_was_uncertain_false() {
        let mock = MockPlatformService::with_config(github_config());
        // Setup PR that will fail to merge but is certain (not uncertain)
        mock.setup_mergeable_pr(1, "feat-a", "Feature A");
        mock.set_merge_response(
            1,
            MergeResult {
                merged: false,
                sha: None,
                message: Some("API error".to_string()),
            },
        );

        let plan = MergePlan {
            steps: vec![MergeStep::Merge {
                bookmark: "feat-a".to_string(),
                pr_number: 1,
                pr_title: "Feature A".to_string(),
                method: MergeMethod::Squash,
                confidence: MergeConfidence::Certain, // Certain, not uncertain
            }],
            bookmarks_to_clear: vec!["feat-a".to_string()],
            rebase_target: None,
            has_actionable: true,
            trunk_branch: "main".to_string(),
        };

        let progress = NoopProgress;
        let result = execute_merge(&plan, &mock, &progress).await.unwrap();

        // Verify: merge failed but was_uncertain is false
        assert!(!result.is_success());
        assert!(!result.was_uncertain); // Should be false for certain merges
        assert_eq!(result.failed_bookmark, Some("feat-a".to_string()));
    }

    #[tokio::test]
    async fn test_execute_merge_calls_retarget() {
        // Test that RetargetBase steps call update_pr_base
        let mock = MockPlatformService::with_config(github_config());
        mock.setup_mergeable_pr(1, "feat-a", "Feature A");
        mock.setup_mergeable_pr(2, "feat-b", "Feature B");

        let plan = MergePlan {
            steps: vec![
                MergeStep::Merge {
                    bookmark: "feat-a".to_string(),
                    pr_number: 1,
                    pr_title: "Feature A".to_string(),
                    method: MergeMethod::Squash,
                    confidence: MergeConfidence::Certain,
                },
                MergeStep::RetargetBase {
                    bookmark: "feat-b".to_string(),
                    pr_number: 2,
                    old_base: "feat-a".to_string(),
                    new_base: "main".to_string(),
                },
                MergeStep::Merge {
                    bookmark: "feat-b".to_string(),
                    pr_number: 2,
                    pr_title: "Feature B".to_string(),
                    method: MergeMethod::Squash,
                    confidence: MergeConfidence::Certain,
                },
            ],
            bookmarks_to_clear: vec!["feat-a".to_string(), "feat-b".to_string()],
            rebase_target: None,
            has_actionable: true,
            trunk_branch: "main".to_string(),
        };

        let progress = NoopProgress;
        let result = execute_merge(&plan, &mock, &progress).await.unwrap();

        // Verify: both merges succeeded
        assert!(result.is_success());
        assert_eq!(result.merged_bookmarks, vec!["feat-a", "feat-b"]);

        // Verify: update_pr_base was called for PR2
        mock.assert_update_base_called(2, "main");
    }

    #[tokio::test]
    async fn test_execute_merge_stops_on_retarget_failure() {
        // Test that retarget failure stops execution
        let mock = MockPlatformService::with_config(github_config());
        mock.setup_mergeable_pr(1, "feat-a", "Feature A");
        mock.setup_mergeable_pr(2, "feat-b", "Feature B");
        // Make the retarget fail
        mock.fail_update_base("API rate limit exceeded");

        let plan = MergePlan {
            steps: vec![
                MergeStep::Merge {
                    bookmark: "feat-a".to_string(),
                    pr_number: 1,
                    pr_title: "Feature A".to_string(),
                    method: MergeMethod::Squash,
                    confidence: MergeConfidence::Certain,
                },
                MergeStep::RetargetBase {
                    bookmark: "feat-b".to_string(),
                    pr_number: 2,
                    old_base: "feat-a".to_string(),
                    new_base: "main".to_string(),
                },
                MergeStep::Merge {
                    bookmark: "feat-b".to_string(),
                    pr_number: 2,
                    pr_title: "Feature B".to_string(),
                    method: MergeMethod::Squash,
                    confidence: MergeConfidence::Certain,
                },
            ],
            bookmarks_to_clear: vec!["feat-a".to_string(), "feat-b".to_string()],
            rebase_target: None,
            has_actionable: true,
            trunk_branch: "main".to_string(),
        };

        let progress = NoopProgress;
        let result = execute_merge(&plan, &mock, &progress).await.unwrap();

        // Verify: first merge succeeded but stopped at retarget failure
        assert!(!result.is_success());
        assert_eq!(result.merged_bookmarks, vec!["feat-a"]); // Only first merged
        assert_eq!(result.failed_bookmark, Some("feat-b".to_string()));
        assert!(result.error_message.as_ref().unwrap().contains("Retarget failed"));
        assert!(!result.was_uncertain); // Retarget failures are not uncertain

        // Verify: merge was called only once (for PR1)
        assert_eq!(mock.merge_call_count(), 1);
    }
}
