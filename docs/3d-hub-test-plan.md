# MakerFoundry 3D: the test plan

**Status:** 2026-10-30. Written before the code: for each P1 issue (`docs/3d-hub-plan.md` §13), the
tests come first, fail, and then the code makes them pass (CLAUDE.md, and VectorCraft's CLAUDE.md
it comes from: orient → plan → test → implement → verify → record → commit).

## How the tests run

- **One command:** `MF_TEST_DATABASE_URL=… cargo xtask ci` (fmt, clippy with the no-panic lints,
  every test against a real Postgres, assets, the Docker build in CI). Database tests skip without
  `MF_TEST_DATABASE_URL`; CI sets it.
- **Where:** readers' unit tests beside their code (`src/model3d/*.rs`, `#[cfg(test)]`); pages and
  flows end to end in `tests/accounts.rs` (a real Postgres schema per test, the stand-in account
  service and storage); every reader of untrusted input in `tests/untrusted_input.rs` (proptest).
- **Fixtures are made in the tests**, from the public specifications (STL, OBJ, 3MF core, ZIP,
  ISO 10303-21), never copied from anywhere: a cube written as binary and ASCII STL, a 3MF
  written by the test's own ZIP writer. No large binary fixtures are committed (VectorCraft's
  test-corpora rule). Real files our creators give us (Nick's Bambu 3MFs) are checked **locally**
  against the readers and never committed without the owner's and the creator's say.
- **The property tests** run 128 cases each in CI; before launch (#27) every reader runs at
  20,000 cases under a 3 GB memory cap (`ulimit -v`), the check that found the SVG memory bug
  (`docs/debug-logs/2026-10-30-svg-arc-memory.md`).
- **Nothing may panic** (workspace lints); every limit is tested at its edge (just under passes,
  just over is refused, with a message and nothing kept).
- **The existing features stay green:** selling, refunds and tax, approval links, founding
  creators, listings, makes, comments: their tests run unchanged on every commit.

## Cross-cutting checks (every issue)

| Check | How |
|---|---|
| No panics on any input | proptest: random bytes and damaged good files for each reader |
| No unbounded allocation | counts and sizes checked before allocating; a claimed huge count refused without allocating |
| Limits at their edges | a test just under and just over each limit |
| Nothing kept on refusal | the stand-in storage is empty after a refused upload |
| Others can't see or change it | another account gets 404 on drafts, edits, deletes |
| Off until switched on | non-admins get 404 while `MF_3D` is off |
| Pages fit a phone | Playwright screenshots at 390 px and 1280 px, `scrollWidth` equal to the width |

## Tests per issue, written first

### #3 MF-35: foundation
- `a_3d_licence_cant_go_on_an_embroidery_design` and the other way round.
- `every_3d_licence_has_a_summary_and_an_official_link`; `every_category_is_accepted_and_nothing_else`.
- `the_3d_section_is_for_admins_until_switched_on` (404 for others, 200 for admins; robots and sitemap
  leave it out while off).
- The migration runs on an existing database with designs in it (every DB test does).

### #4 MF-36: STL
- `a_binary_cube_reads` (12 triangles, 10 mm box); `an_ascii_cube_reads`.
- `a_binary_stl_whose_count_doesnt_match_its_size_is_refused`; `nan_and_infinite_numbers_are_refused`;
  `empty_and_tiny_files_are_refused`; `a_huge_claimed_count_is_refused_without_allocating`;
  `an_html_file_named_stl_is_refused`; `a_solid_word_in_a_binary_header_still_reads_as_binary`.
- proptest `stl_random_bytes`, `stl_damaged`.

### #5 MF-37: OBJ
- `an_obj_cube_with_quads_reads` (faces triangulated); `negative_indexes_resolve`.
- `out_of_range_and_zero_indexes_are_refused`; `too_many_vertices_are_refused`; `other_lines_are_ignored`.
- proptest `obj_random_bytes`, `obj_damaged`.

### #6 MF-38: 3MF
- `a_minimal_3mf_reads` (one cube); `components_and_build_transforms_are_applied`.
- `a_zip_bomb_is_refused` (ratio and total); `too_many_entries_are_refused`;
  `a_path_traversal_name_is_refused`; `a_dtd_or_entity_in_the_model_is_refused`;
  `a_truncated_zip_is_refused`.
- `the_rebuild_keeps_only_known_parts` (an `.exe` and a script dropped);
  `sliced_gcode_is_dropped_but_geometry_and_settings_kept`; `the_rebuild_reads_back_the_same_mesh`.
- `an_embedded_thumbnail_is_found`.
- proptest `threemf_random_bytes`, `threemf_damaged`, `zip_entries_generated`.

### #7 MF-39: STEP and the allow-list
- `a_step_header_passes`; `a_binary_or_html_file_named_step_is_refused`; `an_unknown_extension_is_refused`.
- `step_files_download_as_attachments`.

### #8 MF-40: thumbnails
- `a_cube_renders_to_an_image_of_the_right_size_with_ink_on_it`;
  `a_degenerate_mesh_renders_without_panicking`; `a_huge_mesh_is_reduced_and_renders_in_time`;
  `an_embedded_thumbnail_is_preferred`.

### #9 MF-41: upload
- `a_good_upload_makes_one_draft_with_its_files_and_images`.
- `a_bad_file_a_huge_file_or_a_malformed_3mf_is_refused_and_nothing_is_kept`;
  `too_many_files_are_refused`; `others_cant_see_or_change_a_draft`; `the_daily_limit_holds`.

### #10 MF-42: details, publishing, downloads
- `a_draft_needs_a_title_category_and_licence_to_publish`;
  `published_is_public_unpublished_is_not`; `the_zip_holds_exactly_the_models_files`;
  `downloads_are_counted`; `the_page_has_its_link_preview`.

### #20 MF-52: bulk import
- `thirty_files_across_several_models_group_by_folder` and `…_by_name`
  (endings `_plate2`, `-v2`, `_part3`, `(1)` ignored; images join their stem).
- `a_zip_with_nested_folders_groups_by_folder`; `a_zip_bomb_or_traversal_entry_is_refused`;
  `an_oversized_batch_is_refused`; `a_bad_file_is_reported_and_the_rest_kept`.
- Unit tests of the grouping function on names alone.

### #21 MF-53: import review
- `files_move_between_groups_and_groups_merge_split_and_rename`;
  `a_draft_without_a_licence_cant_publish`; `bulk_publish_needs_the_confirmation`;
  `with_it_every_reviewed_draft_publishes`; `an_origin_address_on_another_site_is_refused`;
  `the_origin_line_shows_on_the_model`; `draft_to_publish_end_to_end`.

### #11 MF-43: Bambu Studio / OrcaSlicer settings
- `a_bambu_3mf_with_ams_reads_printer_nozzle_layers_and_colours_used` (4 filaments, 2 used → 2);
  `an_orca_3mf_reads`; `printer_names_map_to_badges`.
- `malformed_or_huge_settings_keep_the_geometry`; `out_of_range_numbers_are_ignored`;
  `only_whitelisted_keys_are_kept`.
- Fixtures: settings files written in the tests from the documented layout; checked later
  against real files locally.

### #12 MF-44: PrusaSlicer and Cura
- `a_prusaslicer_3mf_reads`; `a_cura_3mf_reads_what_it_can`; `an_unknown_slicer_is_geometry_only`;
  `a_broken_settings_file_never_fails_the_upload`.

### #13 MF-45: print profiles on pages
- `a_model_shows_each_profile_with_badges_and_colours`; `each_profile_downloads_its_own_3mf`;
  `profiles_are_added_and_removed`; `an_unreadable_profile_still_shows_and_downloads`.

### #15 MF-47: remixes
- One test per row of the rules table (plan §6), e.g. `a_remix_of_nd_is_refused`,
  `by_sa_forces_by_sa`, `gpl3_refuses_gpl2`, `cern_w_allows_w_or_s`.
- `two_parents_with_no_common_licence_are_refused_naming_the_parent`;
  `a_creator_may_remix_their_own_all_rights_reserved_model`; `a_cycle_cant_be_made`;
  `the_tree_stops_at_ten_generations`; `a_taken_down_parent_stays_named_without_a_link`.
- Unit tests of the rule function over every pair of licences (exhaustive, it's small).

### #16 MF-48: likes and community
- `likes_count_once_per_account`; `a_make_on_a_3d_model`; `a_3d_model_in_a_collection`;
  `following_a_creator_shows_their_new_models`; `a_reported_comment_reaches_the_queue`;
  `likes_are_in_the_data_download_and_go_with_the_data`.

### #17 MF-49: search
- One test per filter (category, licence, file type, has makes, has a profile, printer,
  multi-colour, filament) and combined; `drafts_and_taken_down_never_appear`;
  `sorting_by_newest_and_popular`.
- `a_filtered_page_under_100ms_with_100000_models` (generated rows; the time printed and
  asserted on the test database).

### #18 MF-50: creator profiles
- `a_creator_with_all_three_kinds`; `counts_are_right`; `drafts_never_shown`;
  `the_founding_badge_shows_only_when_set`.

### #19 MF-51: moderation
- `report_takedown_counter_notice_restore_for_a_3d_model` (files 404 while down);
  `repeat_infringers_are_counted_across_kinds`; one test per rate limit.

### #22 MF-54 (3D part): founding early access
- `a_founding_creator_can_publish_3d_before_the_switch`; `others_cant`;
  `removing_the_badge_removes_early_access`.

### #23 MF-55: showcase consent
- `consent_is_off_by_default_and_nothing_stored`; `ticked_it_is_stored_with_its_time`;
  `withdrawn_keeps_its_time_and_is_no_longer_active`; `deleting_data_withdraws_everything`;
  `nobody_else_can_withdraw_it`.

### #24 MF-56: showcase queue
- `only_opted_in_published_items_are_listed`; `no_email_ever_shows`;
  `the_caption_credits_the_display_name_and_links_the_model`;
  `withdrawal_removes_from_the_queue_and_flags_posted_items`; `the_flag_clears_when_taken_down`.

### #25 MF-57: Also by this maker
- `only_published_designs_are_listed`; `none_when_the_maker_has_no_other_kinds`;
  `there_is_no_get_this_printed_button`.

### #14 MF-46: viewer (after the owner's decision)
- `the_page_works_with_scripts_off`; `the_mesh_endpoint_serves_only_published_or_own`;
  `csp_on_other_pages_is_unchanged`; `the_mesh_format_round_trips`.

### #26 MF-58: legal wording
- Draft only. On merge, the legal-pages tests already check the documents have no owner's
  markers left.

### #27 MF-59: launch
- Every 3D page at phone and desktop width; the property tests at 20,000 cases under a memory
  cap; the whole suite green; the ordering features' tests green.

## The later issues (P2 and P3, the owner's decision 2026-10-30: all of them)

Each is built switched off and tested against a stand-in for the outside service, then switched
on when the owner provides the account or service it needs.

### #28: import from a linked Thingiverse account (official API; needs a registered app)
- A stand-in for Thingiverse's API answering `/users/me` and the user's things.
- `only_the_signed_in_accounts_own_things_are_offered`; `a_token_for_a_cant_import_bs_things`;
  `licence_and_date_are_kept_and_the_origin_link_added`; `the_token_is_never_stored`;
  `off_without_the_app_settings`.

### #29: ClamAV (needs a ClamAV service)
- A stand-in clamd speaking its `INSTREAM` protocol.
- `the_eicar_test_file_is_held`; `a_clean_step_passes`; `the_scanner_being_down_holds_uploads`;
  `off_without_the_setting_and_nothing_changes`.

### #30: showcase posting (needs platform access)
- Stand-in platform APIs.
- `only_active_consents_post`; `withdrawn_before_posting_never_posts`;
  `the_post_credits_only_the_display_name`; `withdrawal_after_posting_flags_or_deletes`.

### #31: email notices (needs a Resend key)
- A stand-in sender.
- `comments_makes_remixes_and_followers_notify_when_on`; `off_by_default`;
  `the_daily_digest_groups_them`; `one_click_unsubscribe_works_without_signing_in`;
  `no_email_address_ever_appears_in_a_notice_to_someone_else`.

### #32: STEP previews (needs a converter service)
- A stand-in converter returning an STL.
- `a_step_file_gets_a_preview_through_the_converter`; `the_converter_failing_keeps_download_only`;
  `its_output_goes_through_the_stl_reader_and_its_limits`.

### #33: contests
- `entries_are_models_or_makes_tagged_before_the_close`; `late_entries_are_refused`;
  `one_vote_per_account`; `winners_show_on_the_contest_page`; `only_admins_create_and_close`.

### #34: points and rewards
- `points_for_publishing_and_makes_with_daily_caps`; `re_uploads_and_self_makes_earn_nothing`;
  `points_are_a_ledger_and_add_up`; `rewards_need_an_admin`.

### #35: parametric models (needs a sandboxed OpenSCAD service)
- A stand-in renderer.
- `parameters_are_read_from_the_scad_and_capped`; `values_outside_their_range_are_refused`;
  `the_output_goes_through_the_stl_reader`; `a_slow_render_times_out_cleanly`.

### #36: public API
- `search_and_a_models_files_and_profiles_as_json`; `keys_are_required_and_rate_limited`;
  `drafts_and_taken_down_models_never_appear`.
