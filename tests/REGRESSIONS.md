# Regression ledger

Every bug fixed on `origin/main` up to v0.1.95 + mesh/claude (`b0eb0eb`, 381 commits), taken from commit messages and bodies,
`CHANGELOG.md` "Fixes" sections, `VALIDATION.md`, `STATUS.md` and `docs/archive/`. They are grouped into regression classes.
Each row gives the fixing commit, the symptom, the root cause and what guards the fix today.

Guard notation:

- `rust:name` is a Rust test. Run it with `cargo test --locked name`, matching on the name only, so the command still works after
  `src/ui/tests.rs` moves.
- `py:file` is a Python fixture: `python3 tests/file.py target/debug/cx`. `[ci]` runs in `ci.yml` on every push, `[rel]` only in
  `release.yml`, and `[local]` in no workflow, so it is run by hand.
- **NEW** marks a guard added with this ledger. Each one was checked red: it fails when its original bug is put back.
- **UNGUARDED** means no automated test fails if the bug comes back. **PARTIAL** means a test touches the area but would not catch
  that exact failure.

`tests/run_regressions.sh` builds cx and runs every guard that works offline on one machine (see the end of this file).

## 1. Mouse wheel and scroll routing

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `16119da` v0.1.6 | Wheel in a managed terminal sent prompt-history arrow keys instead of scrolling the transcript | tmux had no wheel bindings, so the alternate-screen fallback turned the wheel into Up/Down | py:mouse_scrollback_pty [ci] |
| `5aa3123` v0.1.21 | Fullscreen Codex wheel opened tmux's empty `0/0` copy-mode history | stale Codex mouse policy; the fallback entered copy mode even with empty history | py:codex_wheel_pty [rel], py:mouse_scrollback_pty [ci] ("empty 0/0 copy buffer"), py:codex_wheel_live [local] |
| `9c01b9b` v0.1.21 | Wheel fallback was injected into an editor forked by a Codex worker thread | ownership walk read only the leader's `/proc/PID/task/PID/children` | py:codex_wheel_threads_pty [rel] |
| VALIDATION v0.1.21 | Wheel fallback reached read-only observers and same-group child editors | the originating client and read-only state were not checked | py:codex_wheel_pty [rel]; Rust: UNGUARDED |
| `1b281c8` v0.1.29 | One wheel detent moved several rows in Sessions and Files | the terminal's alternate-scroll arrows were accelerated into bursts; mouse reports were not captured directly | rust:list_mouse_navigation_uses_panel_and_modal_ownership, rust:key_release_does_not_advance_selection_twice, py:list_wheel_pty [rel] |
| `8bea0b8` v0.1.29 | Wheel did nothing over a fullscreen PDF opened from the two-location layout | the transfer layout did not register the preview mouse target | rust:fullscreen_pdf_from_two_locations_has_mouse_target |
| `b58b71e` v0.1.29 | Wheel over a confirmation could arm Delete or move the file list underneath | wheel events were not owned by the modal | rust:confirmation_wheel_does_not_arm_delete_or_navigate_files, rust:recursive_delete_warning_is_visible_and_detail_scrolling_cannot_confirm |
| `95a1b7a` v0.1.30 | Clicking a preview did not focus it; the PDF layout jumped while a page loaded | the preview had no click-focus target, and the loading state collapsed the panel | rust:preview_click_focus_and_wheel_keep_file_selection, rust:pending_pdf_keeps_full_workspace_panel |
| `983a246` v0.1.30 | PgDn on a loading PDF jumped ten pages; the bottom overlay overwrote the frame corners | pending page requests were not coalesced; the overlay Block was unbounded | rust:pending_pdf_page_down_remains_single_page, py:preview_search_pty [rel]; frame corners: UNGUARDED |
| `76be106` v0.1.31 | Wheel during file search jumped between matches and wrapped around | wheel reused the n/N match navigation | rust:file_search_wheel_moves_adjacent_rows_without_jumping_matches, py:list_wheel_pty [rel] |
| `dfeb9ef` v0.1.31 | One physical detent sometimes moved two rows | the terminal sends duplicate SGR wheel reports 0–0.02 ms apart (physical trace) | rust:wheel_bursts_keep_direction_position_modifiers_and_keyboard_distinct, rust:physical_wheel_trace_selects_one_adjacent_row_per_tick_in_both_lists |
| `19e7aa9` v0.1.10 | Wheel over a PDF scrolled the browser, not pages; old helpers looped on the page check | the wheel was not scoped to the preview; capability probing was not bounded | rust:pdf_mouse_wheel_is_scoped_to_preview, rust:pdf_scroll_requests_the_next_page, rust:old_helper_page_check_does_not_loop, py:pdf_pages_pty [rel] |
| `cd3f521` v0.1.87 | Opening a session after scrolling landed in read-only scrollback that swallowed input | tmux copy-mode persists across detach; a focus attach did not cancel it | py:focus_exits_copy_mode_pty [ci], py:mouse_scrollback_pty [ci] (contract updated in `8b124cf`) |
| `39a49ba`, `f7c90f3` v0.1.95 | Live previews could not scroll; wheel paging skipped recent history | there was no on-demand history window, and a viewport at the cursor's home row dropped the newest rows | rust:cursor_at_home_does_not_skip_recent_history; scrolling: py:session_preview_live, py:live_session_pane_ui [local] |
| `228b585` v0.1.7 | Very large preview scroll deltas could wrap the offset | `isize` to `i16` cast without clamping | UNGUARDED |

## 2. Preview rendering fidelity

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `d84a340` v0.1.9 | Stale half-block fallback rows stayed visible after the Sixel/Kitty image arrived | the ratatui diff cache was not reset when the smaller native image replaced the fallback | UNGUARDED (visual only) |
| `50d969a` v0.1.9 | A native image encoded for the old font size was drawn after a resize; ASCII confirmations had Unicode borders | the completion was not tied to cell geometry; the ASCII outer border was not used | rust:old_cell_geometry_completion_is_discarded, rust:boxed_confirmation_variant_captures |
| v0.1.9 notes | Transparent images drew black panels | alpha was not mapped to the default cell | rust:transparent_sixel_falls_back_without_black_panels, rust:rich_preview_rejects_bad_images_and_keeps_transparent_pixels_default |
| `228b585` v0.1.7 | Bidi override characters reordered preview and title text | only C0/C1 controls were stripped | rust:terminal_controls_bidi_and_excess_are_removed, rust:hostile_controls_are_not_rendered |
| `d8a484a` v0.1.20 | `G` scrolled into a blank screen; search matched the "Loading" placeholder | the offset used unwrapped line counts, and the placeholder was searchable | rust:preview_g_keeps_last_lines_visible, rust:preview_g_gg_clamp_wrapping_and_resize, rust:preview_loading_does_not_search_placeholder, py:preview_search_pty [rel] |
| v0.1.20 | The selected search result was lost after a resize or refresh | the selection index was not revalidated | rust:preview_search_survives_refresh_and_reveals_on_resize |
| v0.1.25 | Table-like text inside a fenced code block was rendered as a table; the wrong fence closed blocks | fence marker and length were not matched | rust:fence_aliases_mismatched_markers_and_unknown_languages, rust:table_body_stops_before_a_pipe_in_a_code_fence, py:markdown_blocks_pty [rel] |
| v0.1.25 / `072cd46` | Search highlight was misaligned after resizing a table | the cached layout was not keyed by viewport/revision | rust:markdown_table_search_clamps_stored_selection_after_resize |
| v0.1.26 | Wrapped list continuation lines were not aligned under their text | hanging indent ignored Unicode width | rust:markdown_lists_continuations_unicode_and_code_keep_layout, rust:markdown_bullet_wrap_has_hanging_indent, py:markdown_prose_pty [rel] |
| `1cf0e2d` v0.1.26 | Clicking an ordinary search match opened a link picker; heading links lost inline styling | link hit regions came from the underline style; headings skipped inline formatting | rust:markdown_links_math_and_click_picker_are_safe, py:markdown_prose_pty [rel] |
| `d8bdd93` v0.1.27 | A relative link through a symlinked parent opened the wrong file | `..` was normalised lexically instead of on the execution host | rust:parent_traversal_preserves_execution_host_symlink_semantics, py:markdown_relative_pty [rel] |
| `232caba` v0.1.27 | Back from a followed link used stale history in cached locations | link history survived a return to folder navigation | rust:leaving_linked_preview_discards_history_even_in_cached_locations, py:markdown_relative_pty [rel] |
| `8aa4320` v0.1.23 | A clipped matching directory looked identical to a plain directory under NO_COLOR | the match style was colour-only | rust:clipped_directory_find_has_a_distinct_noncolor_modifier |
| v0.1.10 | Rapid PDF paging started overlapping renders; a failed page blanked the preview | no coalescing; failures replaced the last good page | rust:pdf_pages_coalesce_keys_and_preserve_last_page_on_failure |
| `0b2b928` v0.1.90 | Codex prompt backgrounds were too bright; reverse video persisted after SGR 27; DIM was dropped | attribute end sequences were not applied, and the pane palette was ignored | rust:dim_and_attribute_end_sequences_preserve_native_contrast, rust:pane_palette_is_applied_to_both_prompt_background_and_dim_text, rust:prompt_selection_end_restores_normal_foreground_and_muted_background, py:codex_contrast_live [local] |
| `f7c90f3` v0.1.92 | Prompt background blocks stopped at the text; colours broke across rows | trailing cells were not painted, and style was not carried across rows | rust:captured_rows_share_style_and_paint_trailing_cells, py:preview_fidelity_live [local] |
| `88c54f5` v0.1.95 | Every live preview showed "Terminal unavailable" on tmux 3.4 (Ubuntu 24.04) | `#{bracket_paste_flag}` does not exist before tmux 3.5; it expanded to nothing, giving 10 fields | rust:live_view_flags_always_print_a_digit_on_older_tmux |
| `f91242a` v0.1.95 | Live previews vanished once availability returned; hover did not track rendered rows | Info ingestion overwrote the live-preview capabilities; there were no rendered row hit boxes | rust:info_reply_keeps_live_terminal_capability_and_preview_enabled, rust:session_hover_highlights_rendered_row_without_selecting (revamp: hover highlights the rendered row but never selects or fetches, P16) |
| revamp/w2-sessions | Review: typing into a focused preview while scrolled back pinned a CPU core | `session_preview_wait` returned a zero wait while `refresh_session_preview` skipped the request (loaded history page, or no selection) | rust:focused_scrollback_and_missing_selection_never_spin_the_event_loop |
| revamp/w2-sessions | Review: Enter suspended the viewer into `ssh -t` for errored, offline or startup-cache devices | the direct attach did not require a fresh successful listing | rust:unreachable_or_cached_devices_keep_the_in_cx_round_trip |
| revamp/w2-sessions | Review: tapping the copy-mode hint or title in the managed status line detached | `MouseDown1StatusLeft` covers all of status-left, which also held the hint | py:mouse_scrollback_pty (title cell not a detach target; chip tap detaches), rust:managed_detach_is_ctrl_backslash_with_a_tappable_chip |
| revamp/w2-sessions | Review: two devices showed the same `[la]` chip, distinguishable only by colour | the initials took the first two letters of each name | rust:device_initials_stay_unique_without_color |
| `8d978cc` v0.1.72 | Claude lost terminal transparency; old CX left opaque window backgrounds | the managed contrast was set on a solid window style | py:claude_contrast_live [ci] |
| v0.1.69 | Claude question descriptions had low contrast | default-cell backgrounds | py:claude_contrast_live [ci], py:session_color_live [local] |
| `10e4a10` v0.1.5 | The viewer palette was reset after native Fish commands | OSC 104 was emitted by the Fish theme | py:native_command_pty [rel], rust:fish_palette_reset_is_after_config_before_command |
| `26744fc` | Prompt italics were lost in native terminals | tmux terminal overrides | py:shell_return_pty, py:live_pty [local] |
| `7b8d0cd` v0.1.2 | Colours broke on older tmux | RGB capability flags unsupported | py:manual_agents_live [rel] |

## 3. tmux compatibility and inherited TMUX environment

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `5b20243` v0.1.2 | Session creation failed on tmux 2.7 (Rocky 8) | multi-argument `new-session` and `-e` arrived in later tmux | UNGUARDED in automation (manual Rocky 8 docker run in VALIDATION) |
| `a1c4a28` v0.1.2 | Capability checks hung on custom tmux wrappers | probes were not bounded | rust:provider_probe_requires_success_and_bounds_wrappers, rust:exited_probe_wrappers_do_not_leave_background_work |
| `774fb8b` v0.1.3 | The managed launcher broke under a Fish login shell (POSIX quoting parsed by Fish) | tmux ran the launcher through the user's shell | rust:managed_launcher_preserves_fish_palette_quoting |
| `90c9b10` v0.1.19 | Shell activity labels failed on tmux 2.7 | `set-option -p` (pane options) does not exist there; replaced by session options | UNGUARDED (manual Rocky 8 docker run) |
| `64c41ab` v0.1.22 | Updated wheel bindings did not apply after the last session closed; `exit-empty` is unsupported on tmux 2.7 | an empty persistent server kept the old config | py:mouse_scrollback_pty [ci]; tmux 2.7 part UNGUARDED |
| `0e1cfc3` v0.1.14 | cx said tmux was missing on hosts with tmux only in `~/.local/bin` | a noninteractive SSH PATH omits `~/.local/bin` | py:install_fixture [ci] (installer), **NEW** rust:inherited_tmux_environment_never_redirects_default_server_discovery (helper uses `~/.local/bin/tmux` with PATH=/usr/bin:/bin) |
| `343dc07` v0.1.8 | A broken tmux (missing library, failing wrapper) reported sessions as already stopped | any stderr containing "No such file" was treated as "no server running" | py:stop_shell_live [rel], **NEW** rust:absent_tmux_server_is_empty_but_broken_tmux_runtime_is_an_error |
| `06f0d91`, `84822d5`, `2c0f977` v0.1.28 | Inside a managed pane, external discovery listed managed sessions as external (shared `$0`), with duplicate rows and broken remote attach | inherited `TMUX`/`TMUX_PANE` redirected default-server tmux calls | py:tmux_discovery_live [rel], **NEW** rust:inherited_tmux_environment_never_redirects_default_server_discovery |
| `496e3f2` v0.1.71 | Startup readiness broke on legacy tmux | it used a format/flag that legacy tmux lacks | py:terminal_startup_readiness_live [ci] (current tmux only): PARTIAL |
| `11ab3b4`, `e840155` v0.1.79 | The command panel showed "finished" without an exit code; some builds never report it | tmux reports `pane_dead` before `pane_dead_status`, and some builds omit it | py:command_panel_live [ci] |
| `ee14143` v0.1.93 | "persistent tmux user service did not start" after the server died | a stopped transient `cx-tmux-*` unit stays loaded while a `PartOf` pane scope lives, so `systemd-run --unit` refuses the name | rust:only_a_loaded_stopped_tmux_unit_is_restarted, rust:systemd_run_errors_are_summarized_and_bounded |
| incident 2026-10-31 | Every managed session ended in the same second | an agent ran bare `tmux kill-server` inside a cx pane; `$TMUX` targets the managed server even with `TMUX_TMPDIR` set | UNGUARDED (external cause; `ee14143` covers recovery). `run_regressions.sh` unsets TMUX for this reason |
| `dbbd3e0` | The checked attach guard was unreadable when the server environment had an old HOME/XDG_STATE_HOME | tmux `if-shell` inherits the server environment, not the client's | rust:checked_attach_requires_guard_success_and_preserves_external_config |
| `5d7b38f` / release CI | The v0.1.93/94 tags were never published | PTY races on Ubuntu 24.04 (tmux 3.4) runners | fixture fix; lesson: run changed PTY fixtures on tmux 3.4 before tagging |

## 4. Orphan processes, stale units and process lifetime

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| VALIDATION v0.1.9 | 19 orphaned viewers with PPID 1 survived on deleted terminals and ignored SIGTERM | the UI loop busy-polled a hung-up pty | (see `be44093`) |
| `be44093` v0.1.88 | cx spun at 100% CPU after its terminal disappeared | no SIGHUP in its own process group; EOF on the tty was not treated as exit; stop signals were not honoured | py:orphan_exit_pty [ci] |
| `ef1f96e` | Stale restart handoff files accumulated | a handoff was consumed only on a successful restore | rust:failed_restart_handoffs_are_consumed_and_stale_ones_swept, rust:restart_rejects_unsafe_names_symlinks_and_expires |
| `7cb22fb`, `933dae4`, `ef484b3` | Stray viewers and helpers stayed hidden; an orphaned viewer on a live terminal was wrongly stopped | no inventory; classification ignored a live tty | rust:strays_need_evidence_and_daemons_are_kept, rust:scan_sees_this_test_binary_is_not_cx, py:orphan_exit_pty [ci] |
| `ee14143` | stale systemd unit (see class 3) | | rust:only_a_loaded_stopped_tmux_unit_is_restarted |
| `60506e1` v0.1.7 | Shell stop could target an agent or an external terminal | no runtime or provider identity check | rust:stop_shell_confirmation_protects_agents_and_captures_runtime_identity, rust:shell_stop_requires_confirmation_and_rejects_external_and_agent_rows |
| `b5322e5` v0.1.7 | The pane command changed between confirmation and stop | the tmux command was not rechecked at stop time | py:stop_shell_live [rel] |
| `4f8cb9b` | Native-command fixtures killed process groups they did not own | cleanup was too broad | py:native_command_pty [rel] (fixture hygiene) |
| `abb4a39` v0.1.3 | One-shot commands left job groups behind, or broke Fish/Bash/Zsh wrappers | foreground process-group control | py:native_command_pty [rel] |
| `6188285` | The enrollment installer left writer/reader threads blocked | an exited child's inherited pipes were held open | rust:exited_installer_cannot_leave_writer_or_reader_blocked |
| `9238073` | Helper exit with inherited stderr hung cleanup | a descendant kept stderr open | rust:helper_exit_with_inherited_stderr_is_bounded_and_owned_group_is_reaped |
| VALIDATION / `f91242a` | A stalled relay-version or capability check froze adoption | wrapper descendants kept pipes open | rust:relay_version_preflight_is_bounded, rust:relay_version_does_not_wait_for_wrapper_descendants, rust:relay_version_bounds_output_and_requires_success |
| misc | Bounded subprocesses | | rust:bounded_process_timeout_cleans_descendant_and_reader_threads, rust:stalled_terminfo_is_bounded_and_reaped, rust:bash_monitor_mode_is_disabled_for_noninteractive_probe_work |
| `8c110ec` v0.1.77 | A terminal that ended during a move left an intent record | the record was written before the move succeeded | py:adopt_stress_live [local] (needs a ptrace sandbox) |

## 5. Focus, navigation and context

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| VALIDATION (pre-`1f59211`) | Right/l did not enter a folder; Shift+Tab did not reverse focus | input dispatch | rust:horizontal_folder_keys_enter_and_leave_without_previewing_files, rust:reverse_tab_cycles_focus_and_leaves_search |
| `1f59211` | The Files device switch retargeted both panes; an offline host blocked local loading | device selection was global; loading was serialised across hosts | rust:file_device_selection_retargets_only_the_focused_location, rust:device_switch_restores_folder_and_does_not_consume_cut_clipboard, rust:escape_clearing_selection_does_not_discard_inflight_directory_listing, rust:opening_remote_files_from_all_devices_keeps_execution_context_on_enter, py:browser_device_loading_pty [local] |
| VALIDATION | x then t lost the cut intent; pane switches kept the wrong sidebar context | | rust:cut_captures_identity_and_queued_quit_requires_intent, rust:transfer_shortcut_captures_selection_and_opens_destination_picker |
| `6d35290` | Network commands ran against the wrong device scope | the command context was not pinned to the enrolled observer | PARTIAL: rust:palette_network_picker_includes_all_and_scopes_queries_after_choice |
| `b5b95d6`, `c2e6a79` v0.1.13 | The selected-file sidebar followed the wrong pane | the context was not tied to the focused pane | rust:selected_file_context_follows_focused_pane_and_keeps_destination |
| `7082145` v0.1.33 | The per-device Network view listed every observer | the focus scope was ignored | rust:focused_network_only_lists_and_refreshes_the_selected_observer |
| `47bd562` v0.1.47 | Routine navigation produced noisy notifications | navigation notices | UNGUARDED (PARTIAL: rust:notification_types_follow_results_and_typed_inputs_keep_ownership) |
| `803fb78` v0.1.86 | Esc left a half-typed filter applied | the filter was live-applied without a revert point | rust:escaping_a_filter_edit_cancels_it_so_the_next_f_starts_fresh |
| `a4cb275` v0.1.63 | Left Arrow did not cancel the chooser | missing key mapping | PARTIAL: py:session_chooser_pty [ci] (sends Left but only asserts Esc) |
| `3f2ba84` v0.1.63 | The workspace did not open after an up-to-date update check | the startup update path returned early | PARTIAL: py:update_viewer_pty [local] |
| `dac13bc` v0.1.69 | Replies from an old SSH target restored stale sessions after an endpoint edit or removal | host-scoped state was keyed by index | rust:replies_from_previous_ssh_target_cannot_restore_old_sessions_or_containers, rust:endpoint_or_device_index_changes_clear_all_host_scoped_views_and_loading, rust:removal_reindexes_queued_actions_and_never_inherits_old_container_views, rust:reply_for_removed_device_is_ignored_and_reordered_reply_tracks_identity |
| v0.1.69 | The selection jumped during pruning | | rust:container_pruning_preserves_selected_session_identity, rust:background_inventory_reconciliation_preserves_selection_and_invalidates_changed_hosts |
| `c2f0738` | A folder change to a client-only host kept an unavailable profile | the launch profile was not revalidated | rust:moving_to_client_only_host_drops_unavailable_launch_profile |
| `dade746` v0.1.3 | Concurrent session creation | there was no in-flight guard | UNGUARDED |
| v0.1.46 | Queued file actions applied to the wrong host or container after navigation | the scope was not pinned at enqueue time | rust:scoped_mutation_failure_releases_queue_after_navigation_and_preserves_clipboard, rust:rejected_old_host_file_reply_releases_queue_and_endpoint_changes_drop_clipboard |
| v0.1.64 | Device-removal confirmation ignored the arrows, h/l and Enter | modal key ownership | PARTIAL: rust:device_edit_menu_is_remote_only_and_offers_address_or_remove, rust:menu_navigation_counts_edges_and_text_ownership |
| `c60e1a2`, `4513f8d` | The single-direction marker was clipped on long names | marker after the name | rust:device_access_marker_and_toggle_guard_are_clear |
| `d57031d`, `53a7122` v0.1.43–45 | Notices never expired when idle; repeated outage notices | no idle timer; no per-device dedupe | rust:notices_expire_and_repeated_messages_restart_the_timeout, rust:unreachable_notices_are_once_per_device_until_successful_contact, py:notification_timeout_pty, py:unreachable_notice_pty [local] |
| `d53fa86` | Passive reachability failures raised error notices | passive failures were treated as actionable | rust:stale_errors_and_quiet_neighbor_checks_do_not_consume_the_first_device_notice |

## 6. SSH routes, neighbours and transport

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `2dc00fe` | Jump chains could loop or grow without bound | | rust:jump_routes_are_bounded_validated_and_acyclic |
| `de1d0a5` | Native attach and helper calls built different SSH routes | duplicated SSH construction | py:jump_routes_cli [rel] (transport-master: keep a single constructor) |
| `c2e6a79` v0.1.13 | Background jump-host auth prompts; enrollment retargeted a device on an overlapping network | missing BatchMode on routes from personal SSH config; no overlap check | py:jump_routes_cli [rel], py:access_routes_cli [rel] |
| `6188285` v0.1.16 | A queued SSH check outlived its network snapshot | stale queue entries | rust:queued_neighbor_check_does_not_outlive_snapshot |
| `1016401` v0.1.16 | Automatic port checks were unbounded and caused redraws | | rust:network_auto_ssh_is_bounded_cached_and_scoped_without_redraw_work, rust:network_auto_ssh_ignores_stale_unnumeric_and_missing_interface |
| v0.1.13–15 | Addresses sorted as strings; stale ports stayed "open"; a refused connection showed as unknown | | rust:network_numeric_order_stale_port_and_linklocal_enrollment, rust:network_neighbors_sorted_and_unknown_is_honest |
| `17aade9` v0.1.32 | Robot neighbour names were missing; manual Add lost the selected gateway | the resolver output was not validated; the observer was not preserved | rust:resolver_output_is_only_a_valid_name_for_the_requested_address, rust:named_neighbors_are_visible_and_manual_add_preserves_selected_observer, rust:network_device_has_direct_terminal_without_creating_a_tmux_session, py:login_terminal_pty [rel] |
| `bb33fdd` | Enrollment broke when the robot's DHCP lease changed | a DHCP address was pinned instead of the `.local` name | rust:observer_names_stay_names_but_ambiguous_devices_are_rejected (replaced the `17aade9` numeric-IPv4 test) |
| `536853a` v0.1.12 | A closed metadata channel failed listing; raw SSH stderr was shown | there was no single reconnect; errors were unclassified | rust:disconnected_observations_retry_once_but_mutations_never_replay, rust:ssh_failures_are_actionable_and_never_expose_stderr, rust:actual_helper_stderr_is_bounded_and_classified, py:metadata_reconnect_cli [rel] |
| `9238073` | The cached-session remedy was clipped at small widths | | rust:failed_cached_session_keeps_refresh_visible_at_small_widths |
| `d381c3e` v0.1.34 | Robot authentication exceeded the helper deadline | GSSAPI was tried before public keys | UNGUARDED: nothing asserts `PreferredAuthentications=publickey,...` first (src/store.rs, transport-master) |
| `d381c3e` v0.1.34 | Shell attach failed at once on a robot lacking the viewer's terminfo | vendor TERM | py:attach_terminal_pty [rel] |
| VALIDATION v0.1.13 | Identity probe failed on Arch | `hostname` missing; now `uname -n` | UNGUARDED |
| VALIDATION v0.1.7 | Network observation failed on Rocky 8 | `ip` lives in `/usr/sbin`, outside PATH | UNGUARDED (`src/network.rs` candidate list) |
| `8966b68`, `8b20780` v0.1.72 | Pairing claimed bidirectional connectivity from local posture | | py:peer_pairing_cli [ci] |
| `6cdf1b5` v0.1.76 | Managed SSH directories were group-writable under umask 0002 | inherited umask | rust:recovers_only_empty_legacy_root_beneath_private_parent, py:peer_mesh_cli [ci] |
| `260e7da` v0.1.74 | The static build failed on the `renameat2` symbol | libc wrapper missing in musl | build-only (portable.yml musl build) |
| `5fe00ab` v0.1.75 | A redundant background helper update ran during enrollment | | py:enrollment_tooling_cli [rel] |
| `aa51d80` | Shared-LAN rules broke when the boat changed ports; docker0 was masqueraded | rules named interfaces | rust:host_nat_does_not_claim_internet_without_robot_gateway_dns_and_verification (asserts `oifname != "docker0"`), py:sharing_lan_namespace [local] |
| `5bb18e3` | Sharing chose its upstream from stale measurements | | rust:upstream_selection_prefers_throughput_then_latency_and_rejects_stale |
| `f3fecb5` | Mesh teardown used an empty account | the account came only from discovery | py:mesh_util [ci] |

## 7. Session identity, labels and runtime inventory

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `ec586d6` v0.1.7 | An agent's folder came from the tmux shell cwd | it used the shell cwd instead of the agent's workspace | rust:codex_workspace_argument_ignores_prompt_and_config_values, rust:provider_directory_rejects_pid_reuse, py:session_directory_live [local] |
| `c9ed1b8` v0.1.7 | Generated titles never refreshed | | rust:generated_session_names_follow_provider_workspace_and_keep_custom_names |
| v0.1.19 | Renamed sessions reordered the list | | rust:changing_session_labels_preserves_row_order_and_selection |
| `64c41ab` v0.1.22 | A blank saved name gave an empty label | the blank name skipped native lookup | rust:generated_session_names_follow_provider_workspace_and_keep_custom_names |
| `7bec0b9` v0.1.27 | Fresh shells showed a hash or directory label | | py:shell_labels_live [rel], rust:shell_labels_never_include_arguments_or_terminal_controls |
| `12b1069` | Multicall executables were mislabelled | | py:manual_agents_live [rel] |
| `0d5a3c7` v0.1.40 | A host launch offered to resume a container session at the same path | scope was ignored when matching | rust:host_launch_does_not_offer_to_resume_same_path_container_session |
| `6a1477c` v0.1.72 (dup `dfc46ca` not on main) | "Directory unavailable" appeared in Codex titles | unknown status went into the title | rust:unknown_provider_directory_is_not_a_session_title, py:session_names_live [ci] |
| `4930a8b`, `bba6800` v0.1.81 | Unpinned agents showed "Directory unavailable"; switching project did not relabel | no launch-directory fallback | py:session_names_live [ci], py:codex_native_switch_live [local] |
| `71b63b6` v0.1.95 | Duplicate rows for one session; dead runtimes listed as active | identity was keyed by socket path | rust:runtime_inventory_deduplicates_server_aliases, rust:runtime_inventory_omits_dead_panes, rust:runtime_inventory_preserves_distinct_servers, rust:runtime_inventory_rejects_moved_pane_metadata, rust:workspace_duplicate_runtime_reply_is_presented_once |
| `b682fa9` v0.1.69 | Dead retained panes appeared as sessions | | rust:runtime_inventory_omits_dead_panes, py:session_runtime_live [ci] |
| `c12a0c9`, `0e55434` | Pane changes replaced terminal identity; attach could reach a replaced server | | rust:runtime_inventory_terminal_identity_survives_pane_change, rust:terminal_generation_rejects_replaced_or_exited_server, rust:checked_attach_preserves_old_helper_legacy_path, py:session_adversarial_attach [local, root] |
| `f931cc5` v0.1.82 | shell plus YOLO no longer errored | the shell arm returned before the YOLO guard | rust:provider_commands_distinguish_fresh_and_native_resume |
| VALIDATION v0.1.4x | A container agent started in `/` instead of the chosen folder | shell startup changed the cwd | rust:container_agent_restores_requested_folder_after_shell_startup |
| `097e010` v0.1.63 | Stale ANTHROPIC_* environment in new sessions | the tmux server environment was reused | UNGUARDED |

## 8. Outside-terminal adoption

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `88a40ba` v0.1.79 | `cx sessions` and every fixture listed the user's real terminals | the `/proc` scan was folded into `Operation::Sessions` | **NEW** rust:sessions_stay_managed_only_while_outside_terminals_use_their_own_operation |
| `d5a38dd` v0.1.78 | The viewer's own terminal was offered for adoption | | rust:a_cx_viewer_terminal_is_never_listed |
| `cb7413c` v0.1.80 | Ancestor terminals and relays were listed; a CX terminal was labelled shell | | rust:excludes_managed_relays_and_the_viewer_terminal |
| `1b728b0` | A moved terminal was re-listed; the empty source window stayed open | the session leader was still seen | rust:pairs_leaders_with_master_holders_and_classifies_them, py:adopt_terminal_live [local] |
| `b9e0bf6` v0.1.83 | A failed or remote move made the terminal vanish | the record was written before the launch command was ready | py:adopt_sudo_live [local]; CI: UNGUARDED |
| `21632fc` v0.1.84 | "unsupported artifact member" when enabling terminal moving | the member allowlist lacked `cx-reptyr` | **NEW** rust:relay_and_portable_tool_members_extract_but_other_members_are_refused |
| `538baee` v0.1.82 | "SSH access unavailable" notices came from background discovery | errors were not marked best-effort | UNGUARDED |
| `f91242a` | Adoption stalled after availability returned | needed a second Enter | rust:adoption_continues_after_availability_without_second_enter |

## 9. Files, transfers, updater and installer

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `42315b2` | Preview opened FIFOs and devices (hang, hardware side effects) | the file type was checked after `open` | **NEW** rust:previews_refuse_fifos_and_devices_without_opening_them |
| `abe489d`, `59931a1`, `b91b8df` | Transfer writes could follow swapped directories | | rust:anchored_directory_swap_never_redirects_writes, rust:changed_identity_and_symlink_ancestors_prevent_deletion |
| VALIDATION (transfers) | Skipped-copy deletion risk, stale receipts, key collisions, lost retry state | | tests/transfers.rs suite, py:transfer_retry_pty [local] |
| v0.1.46 | Cross-container cut cleanup removed unverified files | | rust:tree_move_directory_cleanup_is_empty_only_and_inode_guarded |
| `7bec0b9` v0.1.27 | Updater failed on a reused inode backup name | exclusive-only allocation | rust:source_mismatch_rollback_collision_and_concurrent_locks |
| `7082145` v0.1.33 | The update check failed under the GitHub API rate limit | | rust:exhausted_api_quota_uses_official_tag_and_keeps_signature_gate, rust:release_page_redirect_rejects_downgrades_prereleases_and_other_origins |
| `8059e96` v0.1.2 | Bootstrap misparsed compact GitHub JSON | | py:install_fixture [ci] |
| `4fb96ab` | RHEL curl-minimal was replaced | | py:distro_install_fixture (portable.yml) |
| `0e39b73` v0.1.66 | Devcontainer rebuild was reported as failed | build progress exhausted the output budget | rust:workspace_progress_does_not_exhaust_result_budget |
| `4829ab6` v0.1.86 | Container discovery took seconds (one inspect per container) | serial `docker inspect`; fallback needed | py:container_discovery_fixtures [ci] (fallback path) |
| `357c9d4` v0.1.4 | `:` asked for a helper upgrade on supported hosts | the viewer filtered Info capabilities down to agent names | rust:real_info_response_retains_native_command_capability |
| `36cd089`, `7fba202` v0.1.3–4 | Ctrl+C/Ctrl+Z and builtins misbehaved in POSIX shell profiles | | py:native_command_pty [rel] (also CX_TEST_SHELL=/bin/sh) |
| `20a3737` v0.1.85 | Stale "Connection closed" text showed on session handoff | LeaveAlternateScreen restored the old primary buffer | py:terminal_startup_loader_pty [ci] (raw `\e[?1049l\e[2J`) |
| `2acff83` v0.1.11 | OSC 52 copies never reached the viewer | | py:clipboard_pty [rel] |

## Test-infrastructure regressions (fixture or CI only)

`0e1cfc3` (mouse fixture race), `2b71421`, `072cd46` (partial frame reads), `4c3c3f0`, `78986ca`, `9a5add9` (no `~/.cache` on clean
runners), `e840155`, `ca94e9d`, `34a7d51` (ratatui split-cell redraws), `0626261`, `efd535e` (ptrace in CI), `a381eae`, `5d7b38f`
(Enter before live preview), `8b124cf` (contract change), `2f99a6b` (rustfmt drift), `61185b1`, `e9622b6`, `bba6800` (stale
expectations), `5511207` (CI zsh prompts), `7500697`, `eb2f15b` (release build). Recurring lessons:

- Wait for completed frames, never for a substring of a diff redraw.
- Stop every managed server a fixture starts. `folder_command_ui_pty` left a running transient `cx-tmux-<sha>` user unit behind
  its deleted temporary state on every run (found while building this ledger). It is fixed here: the fixture now kills its own
  socket's server in `finally`. Check with `systemctl --user list-units 'cx-tmux-*'` before and after a fixture.
- Pop TMUX/TMUX_PANE in every fixture. `session_directory_live` and `native_command_pty` still pass `os.environ`, including an
  inherited TMUX, through to cx. Their own tmux calls use explicit `-S` sockets, so they are safe today, but any new bare tmux
  call would target the caller's server.

## Remaining unguarded, by value

1. `d381c3e`: SSH option order (publickey before GSSAPI). Belongs to transport-master; add a store/transport unit test on
   `store::ssh()` args.
2. `5b20243`, `90c9b10`, `64c41ab`, `496e3f2`: tmux 2.7–3.4 compatibility. The only coverage is manual docker runs. Suggest a
   fake-tmux "2.7 personality" that rejects `set-option -p`, `new-session -e` and `exit-empty`, or an `ubuntu:24.04` /
   `rockylinux:8` CI job.
3. `097e010`: Anthropic environment refresh before create and reset.
4. `dade746`: concurrent session-creation guard. `47bd562`: notification quietness. `538baee`: quiet external-discovery errors
   (UI behaviour; add after ui-split).
5. `d84a340`, `983a246`: native image fallback-cell clearing and overlay frame corners. Visual; needs a capture test after the
   redesign.
6. `b9e0bf6`, `8c110ec`, `1b728b0`: adoption flows covered only by local ptrace fixtures.
7. Rocky `ip` in sbin, Arch without `hostname`: host-portability probes.

## Must-retest after the UI revamp

Run everything at once with `tests/run_regressions.sh` (add `--with-ssh` and `--with-docker` where available). One by one:

Rust UI guards (in `src/ui/tests.rs` today; the name filter survives the move):

```
cargo test --locked list_mouse_navigation_uses_panel_and_modal_ownership
cargo test --locked key_release_does_not_advance_selection_twice
cargo test --locked fullscreen_pdf_from_two_locations_has_mouse_target
cargo test --locked confirmation_wheel_does_not_arm_delete_or_navigate_files
cargo test --locked recursive_delete_warning_is_visible_and_detail_scrolling_cannot_confirm
cargo test --locked preview_click_focus_and_wheel_keep_file_selection
cargo test --locked pending_pdf_keeps_full_workspace_panel
cargo test --locked pending_pdf_page_down_remains_single_page
cargo test --locked file_search_wheel_moves_adjacent_rows_without_jumping_matches
cargo test --locked wheel_bursts_keep_direction_position_modifiers_and_keyboard_distinct
cargo test --locked physical_wheel_trace_selects_one_adjacent_row_per_tick_in_both_lists
cargo test --locked pdf_mouse_wheel_is_scoped_to_preview
cargo test --locked pdf_scroll_requests_the_next_page
cargo test --locked old_helper_page_check_does_not_loop
cargo test --locked pdf_pages_coalesce_keys_and_preserve_last_page_on_failure
cargo test --locked old_cell_geometry_completion_is_discarded
cargo test --locked boxed_confirmation_variant_captures
cargo test --locked preview_g_keeps_last_lines_visible
cargo test --locked preview_g_gg_clamp_wrapping_and_resize
cargo test --locked preview_loading_does_not_search_placeholder
cargo test --locked preview_search_survives_refresh_and_reveals_on_resize
cargo test --locked markdown_table_search_clamps_stored_selection_after_resize
cargo test --locked markdown_links_math_and_click_picker_are_safe
cargo test --locked leaving_linked_preview_discards_history_even_in_cached_locations
cargo test --locked clipped_directory_find_has_a_distinct_noncolor_modifier
cargo test --locked info_reply_keeps_live_terminal_capability_and_preview_enabled
cargo test --locked session_hover_highlights_rendered_row_without_selecting
cargo test --locked focused_scrollback_and_missing_selection_never_spin_the_event_loop
cargo test --locked unreachable_or_cached_devices_keep_the_in_cx_round_trip
cargo test --locked device_initials_stay_unique_without_color
cargo test --locked horizontal_folder_keys_enter_and_leave_without_previewing_files
cargo test --locked reverse_tab_cycles_focus_and_leaves_search
cargo test --locked file_device_selection_retargets_only_the_focused_location
cargo test --locked device_switch_restores_folder_and_does_not_consume_cut_clipboard
cargo test --locked escape_clearing_selection_does_not_discard_inflight_directory_listing
cargo test --locked opening_remote_files_from_all_devices_keeps_execution_context_on_enter
cargo test --locked selected_file_context_follows_focused_pane_and_keeps_destination
cargo test --locked focused_network_only_lists_and_refreshes_the_selected_observer
cargo test --locked escaping_a_filter_edit_cancels_it_so_the_next_f_starts_fresh
cargo test --locked replies_from_previous_ssh_target_cannot_restore_old_sessions_or_containers
cargo test --locked endpoint_or_device_index_changes_clear_all_host_scoped_views_and_loading
cargo test --locked removal_reindexes_queued_actions_and_never_inherits_old_container_views
cargo test --locked reply_for_removed_device_is_ignored_and_reordered_reply_tracks_identity
cargo test --locked container_pruning_preserves_selected_session_identity
cargo test --locked moving_to_client_only_host_drops_unavailable_launch_profile
cargo test --locked host_launch_does_not_offer_to_resume_same_path_container_session
cargo test --locked queued_neighbor_check_does_not_outlive_snapshot
cargo test --locked network_auto_ssh_is_bounded_cached_and_scoped_without_redraw_work
cargo test --locked failed_cached_session_keeps_refresh_visible_at_small_widths
cargo test --locked notices_expire_and_repeated_messages_restart_the_timeout
cargo test --locked unreachable_notices_are_once_per_device_until_successful_contact
cargo test --locked failed_restart_handoffs_are_consumed_and_stale_ones_swept
cargo test --locked real_info_response_retains_native_command_capability
cargo test --locked adoption_continues_after_availability_without_second_enter
cargo test --locked device_access_marker_and_toggle_guard_are_clear
```

PTY and viewer fixtures (`cargo build --locked` first):

```
python3 tests/list_wheel_pty.py target/debug/cx
python3 tests/preview_search_pty.py target/debug/cx
python3 tests/pdf_pages_pty.py target/debug/cx
python3 tests/browser_find_pty.py target/debug/cx
python3 tests/markdown_preview_pty.py target/debug/cx
python3 tests/markdown_blocks_pty.py target/debug/cx
python3 tests/markdown_prose_pty.py target/debug/cx
python3 tests/markdown_relative_pty.py target/debug/cx
python3 tests/source_preview_pty.py target/debug/cx
python3 tests/session_chooser_pty.py target/debug/cx
python3 tests/startup_directory_pty.py target/debug/cx
python3 tests/terminal_startup_loader_pty.py target/debug/cx
python3 tests/transfer_enter_pty.py target/debug/cx
python3 tests/folder_command_ui_pty.py target/debug/cx
python3 tests/orphan_exit_pty.py target/debug/cx
python3 tests/focus_exits_copy_mode_pty.py target/debug/cx
python3 tests/mouse_scrollback_pty.py target/debug/cx
python3 tests/codex_wheel_pty.py target/debug/cx
python3 tests/codex_wheel_threads_pty.py target/debug/cx
python3 tests/enrollment_auth_pty.py target/debug/cx
python3 tests/browser_device_loading_pty.py target/debug/cx
python3 tests/notification_timeout_pty.py target/debug/cx
python3 tests/unreachable_notice_pty.py target/debug/cx
python3 tests/update_viewer_pty.py target/debug/cx
python3 tests/transfer_retry_pty.py target/debug/cx
```

UI-independent guards that must not need edits (if they break, the revamp changed behaviour, not wording):
`cargo test --locked --test regressions`, `tests/session_core.rs`, `tests/transfers.rs`, `tests/files_network.rs`, and the
helper/CLI fixtures (`*_live.py`, `*_cli.py`, `container_discovery_fixtures.py`, `preview_helper_live.py`, `pdf_pages_helper.py`).

## PTY fixtures coupled to UI text

These drive the ratatui viewer (`cx restart <state>` or a bare `cx`) and assert or wait on rendered words, panel titles, key hints
or exact cell positions through pyte. Expect to update them when the UI is redesigned. Keep the behaviour each one asserts and
change only the locators.

- **Heavy (screen text, labels and positions):** `session_chooser_pty` ("[s] Shell", bold selection), `list_wheel_pty`,
  `browser_find_pty`, `browser_ecosystem_pty`, `browser_device_loading_pty`, `browser_devices_live_pty`, `preview_search_pty`,
  `pdf_pages_pty`, `markdown_preview_pty`, `markdown_blocks_pty`, `markdown_prose_pty`, `markdown_relative_pty`,
  `source_preview_pty`, `transfer_enter_pty`, `transfer_retry_pty`, `notification_timeout_pty` (bottom-right position),
  `unreachable_notice_pty`, `update_viewer_pty`, `enrollment_auth_pty`, `workspace_flows_pty`, `workspace_live_jobs_pty`,
  `provider_choices_pty`, `session_adversarial_ui`, `live_session_pane_ui` ("New shell", "Scrollback"), `capture_ui`.
- **Moderate (a few labels or hints):** `folder_command_ui_pty` ("Actions", "Run command here", "Esc closes panel"),
  `startup_directory_pty`, `terminal_startup_loader_pty` (loader text; its raw-byte handoff assertion is not text-coupled),
  `remote_folder_command_pty`, `workspace_pty`, `shell_return_pty` ("Work"), `preview_fidelity_live`, `session_color_live`.
- **Light (start the viewer but assert process behaviour):** `orphan_exit_pty` (waits for "Work"/"Workspace" before killing the
  terminal).
- **Not coupled (tmux, helper or CLI only):** `mouse_scrollback_pty`, `codex_wheel_pty`, `codex_wheel_threads_pty`,
  `focus_exits_copy_mode_pty`, `clipboard_pty`, `attach_terminal_pty`, `login_terminal_pty`, `native_command_pty`,
  `live_pty`, and all `*_live.py`, `*_cli.py` and `*_fixture(s).py` not listed above.

## Running the offline set

```
tests/run_regressions.sh                 # fmt + full cargo test + offline fixtures (tmux, python3, pyte)
tests/run_regressions.sh --quick         # only the black-box Rust regression guards + fixtures
tests/run_regressions.sh --with-ssh --with-docker
tests/run_regressions.sh --only list_wheel_pty mouse_scrollback_pty --no-rust
CX_PYTHONPATH=/path/with/pyte:/path/with/wcwidth tests/run_regressions.sh   # when pyte is not installed
```

The script unsets TMUX/TMUX_PANE and uses a private TMUX_TMPDIR. A fixture can therefore never reach the caller's managed or
personal tmux server, even when the script runs inside a cx pane. Fixtures with an unmet requirement (pyte, fish, poppler, sshd,
docker) are reported as SKIP, never as PASS.
