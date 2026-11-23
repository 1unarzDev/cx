# Regression ledger

Every bug fixed on `origin/main` up to v0.1.95 + mesh/claude (`b0eb0eb`, 381 commits), taken from commit messages and bodies,
`CHANGELOG.md` "Fixes" sections, `VALIDATION.md`, `STATUS.md` and `docs/archive/`. They are grouped into regression classes.
Each row gives the fixing commit, the symptom, the root cause and what guards the fix today.

Guard notation:

- `rust:name` is a Rust test. Run it with `cargo test --locked name`, matching on the name only, so the command still works after
  `src/ui/tests.rs` moves.
- `py:file` is a Python fixture: `python3 tests/file.py target/debug/cx`. `[ci]` is in the `tests/run_regressions.sh` list, which
  `ci.yml` (every push) and `release.yml` (every tag) run; `[local]` is in no workflow, so it is run by hand.
- **NEW** marks a guard added with this ledger. Each one was checked red: it fails when its original bug is put back.
- **UNGUARDED** means no automated test fails if the bug comes back. **PARTIAL** means a test touches the area but would not catch
  that exact failure.

`tests/run_regressions.sh` builds cx and runs every guard that works offline on one machine (see the end of this file).

## 1. Mouse wheel and scroll routing

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `16119da` v0.1.6 | Wheel in a managed terminal sent prompt-history arrow keys instead of scrolling the transcript | tmux had no wheel bindings, so the alternate-screen fallback turned the wheel into Up/Down | py:mouse_scrollback_pty [ci] |
| `5aa3123` v0.1.21 | Fullscreen Codex wheel opened tmux's empty `0/0` copy-mode history | stale Codex mouse policy; the fallback entered copy mode even with empty history | py:codex_wheel_pty [ci], py:mouse_scrollback_pty [ci] ("empty 0/0 copy buffer"), py:codex_wheel_live [local] |
| `9c01b9b` v0.1.21 | Wheel fallback was injected into an editor forked by a Codex worker thread | ownership walk read only the leader's `/proc/PID/task/PID/children` | py:codex_wheel_threads_pty [ci] |
| VALIDATION v0.1.21 | Wheel fallback reached read-only observers and same-group child editors | the originating client and read-only state were not checked | py:codex_wheel_pty [ci]; Rust: UNGUARDED |
| `1b281c8` v0.1.29 | One wheel detent moved several rows in Sessions and Files | the terminal's alternate-scroll arrows were accelerated into bursts; mouse reports were not captured directly | rust:list_mouse_navigation_uses_panel_and_modal_ownership, rust:key_release_does_not_advance_selection_twice, py:list_wheel_pty [ci] |
| `8bea0b8` v0.1.29 | Wheel did nothing over a fullscreen PDF opened from the two-location layout | the transfer layout did not register the preview mouse target | rust:fullscreen_pdf_from_two_locations_has_mouse_target |
| `b58b71e` v0.1.29 | Wheel over a confirmation could arm Delete or move the file list underneath | wheel events were not owned by the modal | rust:confirmation_wheel_does_not_arm_delete_or_navigate_files, rust:recursive_delete_warning_is_visible_and_detail_scrolling_cannot_confirm |
| `95a1b7a` v0.1.30 | Clicking a preview did not focus it; the PDF layout jumped while a page loaded | the preview had no click-focus target, and the loading state collapsed the panel | rust:preview_click_focus_and_wheel_keep_file_selection, rust:pending_pdf_keeps_full_workspace_panel |
| `983a246` v0.1.30 | PgDn on a loading PDF jumped ten pages; the bottom overlay overwrote the frame corners | pending page requests were not coalesced; the overlay Block was unbounded | rust:pending_pdf_page_down_remains_single_page, py:preview_search_pty [ci]; frame corners: UNGUARDED |
| revamp nav | Hover fought the keyboard for the selection; moving in Devices refreshed and reset the list (P16/P17) | `MouseEventKind::Moved` selected rows; `Devices::move_by` called `refresh()` | rust:hover_and_motion_never_select_or_redraw, rust:hub_moves_never_refresh_and_keep_the_session_by_identity (the Devices rail became the hub map), rust:device_slots_attach_and_moving_clamps_without_wrapping (session lists became tiles: no scrolloff to keep) |
| `76be106` v0.1.31 | Wheel during file search jumped between matches and wrapped around | wheel reused the n/N match navigation | rust:file_search_wheel_moves_adjacent_rows_without_jumping_matches, py:list_wheel_pty [ci] |
| `dfeb9ef` v0.1.31 | One physical detent sometimes moved two rows | the terminal sends duplicate SGR wheel reports 0–0.02 ms apart (physical trace) | rust:wheel_bursts_keep_direction_position_modifiers_and_keyboard_distinct, rust:physical_wheel_trace_selects_one_adjacent_row_per_tick_in_both_lists |
| `19e7aa9` v0.1.10 | Wheel over a PDF scrolled the browser, not pages; old helpers looped on the page check | the wheel was not scoped to the preview; capability probing was not bounded | rust:pdf_mouse_wheel_is_scoped_to_preview, rust:pdf_scroll_requests_the_next_page, rust:old_helper_page_check_does_not_loop, py:pdf_pages_pty [ci] |
| `cd3f521` v0.1.87 | Opening a session after scrolling landed in read-only scrollback that swallowed input | tmux copy-mode persists across detach; a focus attach did not cancel it | py:focus_exits_copy_mode_pty [ci], py:mouse_scrollback_pty [ci] (contract updated in `8b124cf`) |
| `39a49ba`, `f7c90f3` v0.1.95 | Live previews could not scroll; wheel paging skipped recent history | there was no on-demand history window, and a viewport at the cursor's home row dropped the newest rows | rust:cursor_at_home_does_not_skip_recent_history; scrolling: py:session_preview_live, py:live_session_pane_ui [local] |
| `228b585` v0.1.7 | Very large preview scroll deltas could wrap the offset | `isize` to `i16` cast without clamping | UNGUARDED |
| v0.5.0 (050-agents) | (Guard for new agents) the wheel in Antigravity (`agy`) or opencode must scroll the agent's own transcript, never open tmux copy-mode or send arrow keys; with their mouse off the stale-policy SGR fallback (Codex-only) must not fire for them | agy is started full screen (`--settings '{"altScreenMode":"always"}'`) and opencode with mouse capture (`OPENCODE_CLI_CONFIG_CONTENT='{"mouse":true}'`); `wheel_owner` reads the table's `sgr_wheel_fallback` (Codex only) | **NEW** py:agent_wheel_pty [ci] (red with agy's fallback set true: "stale-mouse wheel sent bytes"), rust:only_codex_takes_the_stale_wheel_fallback, rust:agy_and_opencode_commands_come_from_the_table |

## 2. Preview rendering fidelity

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `d84a340` v0.1.9 | Stale half-block fallback rows stayed visible after the Sixel/Kitty image arrived | the ratatui diff cache was not reset when the smaller native image replaced the fallback | UNGUARDED (visual only) |
| `50d969a` v0.1.9 | A native image encoded for the old font size was drawn after a resize; ASCII confirmations had Unicode borders | the completion was not tied to cell geometry; the ASCII outer border was not used | rust:old_cell_geometry_completion_is_discarded, rust:boxed_confirmation_variant_captures |
| v0.1.9 notes | Transparent images drew black panels | alpha was not mapped to the default cell | rust:transparent_sixel_falls_back_without_black_panels, rust:rich_preview_rejects_bad_images_and_keeps_transparent_pixels_default |
| `228b585` v0.1.7 | Bidi override characters reordered preview and title text | only C0/C1 controls were stripped | rust:terminal_controls_bidi_and_excess_are_removed, rust:hostile_controls_are_not_rendered |
| `d8a484a` v0.1.20 | `G` scrolled into a blank screen; search matched the "Loading" placeholder | the offset used unwrapped line counts, and the placeholder was searchable | rust:preview_g_keeps_last_lines_visible, rust:preview_g_gg_clamp_wrapping_and_resize, rust:preview_loading_does_not_search_placeholder, py:preview_search_pty [ci] |
| v0.1.20 | The selected search result was lost after a resize or refresh | the selection index was not revalidated | rust:preview_search_survives_refresh_and_reveals_on_resize |
| v0.1.25 | Table-like text inside a fenced code block was rendered as a table; the wrong fence closed blocks | fence marker and length were not matched | rust:fence_aliases_mismatched_markers_and_unknown_languages, rust:table_body_stops_before_a_pipe_in_a_code_fence, py:markdown_blocks_pty [ci] |
| v0.1.25 / `072cd46` | Search highlight was misaligned after resizing a table | the cached layout was not keyed by viewport/revision | rust:markdown_table_search_clamps_stored_selection_after_resize |
| v0.1.26 | Wrapped list continuation lines were not aligned under their text | hanging indent ignored Unicode width | rust:markdown_lists_continuations_unicode_and_code_keep_layout, rust:markdown_bullet_wrap_has_hanging_indent, py:markdown_prose_pty [ci] |
| `1cf0e2d` v0.1.26 | Clicking an ordinary search match opened a link picker; heading links lost inline styling | link hit regions came from the underline style; headings skipped inline formatting | rust:markdown_links_math_and_click_picker_are_safe, py:markdown_prose_pty [ci] |
| `d8bdd93` v0.1.27 | A relative link through a symlinked parent opened the wrong file | `..` was normalised lexically instead of on the execution host | rust:parent_traversal_preserves_execution_host_symlink_semantics, py:markdown_relative_pty [ci] |
| `232caba` v0.1.27 | Back from a followed link used stale history in cached locations | link history survived a return to folder navigation | rust:leaving_linked_preview_discards_history_even_in_cached_locations, py:markdown_relative_pty [ci] |
| `8aa4320` v0.1.23 | A clipped matching directory looked identical to a plain directory under NO_COLOR | the match style was colour-only | rust:clipped_directory_find_has_a_distinct_noncolor_modifier |
| v0.1.10 | Rapid PDF paging started overlapping renders; a failed page blanked the preview | no coalescing; failures replaced the last good page | rust:pdf_pages_coalesce_keys_and_preserve_last_page_on_failure |
| `0b2b928` v0.1.90 | Codex prompt backgrounds were too bright; reverse video persisted after SGR 27; DIM was dropped | attribute end sequences were not applied, and the pane palette was ignored | rust:dim_and_attribute_end_sequences_preserve_native_contrast, rust:pane_palette_is_applied_to_both_prompt_background_and_dim_text, rust:prompt_selection_end_restores_normal_foreground_and_muted_background, py:codex_contrast_live [local] |
| `f7c90f3` v0.1.92 | Prompt background blocks stopped at the text; colours broke across rows | trailing cells were not painted, and style was not carried across rows | rust:captured_rows_share_style_and_paint_trailing_cells, py:preview_fidelity_live [local] |
| `88c54f5` v0.1.95 | Every live preview showed "Terminal unavailable" on tmux 3.4 (Ubuntu 24.04) | `#{bracket_paste_flag}` does not exist before tmux 3.5; it expanded to nothing, giving 10 fields | rust:live_view_flags_always_print_a_digit_on_older_tmux |
| `f91242a` v0.1.95 | Live previews vanished once availability returned; hover did not select rows | Info ingestion overwrote the live-preview capabilities; there were no rendered row hit boxes | rust:info_reply_keeps_live_terminal_capability_and_preview_enabled, rust:session_hover_selects_rendered_row_without_opening (revamp: hover never selects, spec P16; the guard now asserts a click selects the rendered row through the hit map and a second tap opens) |
| revamp/w2-sessions | Review: typing into a focused preview while scrolled back pinned a CPU core | `session_preview_wait` returned a zero wait while `refresh_session_preview` skipped the request (loaded history page, or no selection) | rust:focused_scrollback_and_missing_selection_never_spin_the_event_loop |
| revamp/w2-sessions | Review: Enter suspended the viewer into `ssh -t` for errored, offline or startup-cache devices | the direct attach did not require a fresh successful listing | rust:unreachable_or_cached_devices_keep_the_in_cx_round_trip (since 050-dead an errored device is not tried at all: Enter only says why) |
| revamp/w2-sessions | Review: tapping the copy-mode hint or title in the managed status line detached | `MouseDown1StatusLeft` covers all of status-left, which also held the hint | py:mouse_scrollback_pty (title cell not a detach target; chip tap detaches), rust:managed_detach_is_ctrl_backslash_with_a_tappable_chip |
| revamp/w2-sessions | Review: two devices showed the same `[la]` chip, distinguishable only by colour | the initials took the first two letters of each name | rust:device_initials_stay_unique_without_color (now the device letters of `src/ui/hotkeys.rs`) |
| revamp/w2-sessions | tmux < 3.0 rejected the managed config (status-left mouse binding, `show-options` without `-w`) and inline input key names | tmux 2.7 lacks `MouseDown1StatusLeft`, window options need `-w`, older key names | rust:managed_detach_is_ctrl_backslash_with_a_tappable_chip (string rewrite only); real tmux 2.7 is manual: `python tests/legacy_tmux_run.py TMUX27/bin tests/run_regressions.sh --no-rust --only ctrl_bracket_attach_pty list_wheel_pty` with a locally built tmux 2.7 |
| revamp/w2-sessions | Review: a remote attach ignored the viewer's CX_ASCII, showing `⏏` glyphs | ssh forwards TERM but not CX_ASCII; the host read its own environment | rust:checked_attach_carries_the_viewers_ascii_switch_only_when_set |
| revamp/w2-sessions | Review: there was no way to send a literal Ctrl+\ once it always detaches | no palette action | rust:palette_sends_a_literal_ctrl_backslash_to_the_session |
| revamp/w2-sessions | Review: clicking the preview while search was open sent the typed search to the shell | preview click guard ignored open inputs | rust:preview_clicks_leave_an_open_text_input_in_charge |
| `8d978cc` v0.1.72 | Claude lost terminal transparency; old CX left opaque window backgrounds | the managed contrast was set on a solid window style | py:claude_contrast_live [ci] |
| v0.1.69 | Claude question descriptions had low contrast | default-cell backgrounds | py:claude_contrast_live [ci], py:session_color_live [local] |
| `10e4a10` v0.1.5 | The viewer palette was reset after native Fish commands | OSC 104 was emitted by the Fish theme | py:native_command_pty [ci], rust:fish_palette_reset_is_after_config_before_command |
| `26744fc` | Prompt italics were lost in native terminals | tmux terminal overrides | py:shell_return_pty, py:live_pty [local] |
| `7b8d0cd` v0.1.2 | Colours broke on older tmux | RGB capability flags unsupported | py:manual_agents_live [ci] |

## 3. tmux compatibility and inherited TMUX environment

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `5b20243` v0.1.2 | Session creation failed on tmux 2.7 (Rocky 8) | multi-argument `new-session` and `-e` arrived in later tmux | UNGUARDED in automation (manual Rocky 8 docker run in VALIDATION) |
| `a1c4a28` v0.1.2 | Capability checks hung on custom tmux wrappers | probes were not bounded | rust:provider_probe_requires_success_and_bounds_wrappers, rust:exited_probe_wrappers_do_not_leave_background_work |
| `774fb8b` v0.1.3 | The managed launcher broke under a Fish login shell (POSIX quoting parsed by Fish) | tmux ran the launcher through the user's shell | rust:managed_launcher_preserves_fish_palette_quoting |
| `90c9b10` v0.1.19 | Shell activity labels failed on tmux 2.7 | `set-option -p` (pane options) does not exist there; replaced by session options | UNGUARDED (manual Rocky 8 docker run) |
| `64c41ab` v0.1.22 | Updated wheel bindings did not apply after the last session closed; `exit-empty` is unsupported on tmux 2.7 | an empty persistent server kept the old config | py:mouse_scrollback_pty [ci]; tmux 2.7 part UNGUARDED |
| `0e1cfc3` v0.1.14 | cx said tmux was missing on hosts with tmux only in `~/.local/bin` | a noninteractive SSH PATH omits `~/.local/bin` | py:install_fixture [ci] (installer), **NEW** rust:inherited_tmux_environment_never_redirects_default_server_discovery (helper uses `~/.local/bin/tmux` with PATH=/usr/bin:/bin) |
| `343dc07` v0.1.8 | A broken tmux (missing library, failing wrapper) reported sessions as already stopped | any stderr containing "No such file" was treated as "no server running" | py:stop_shell_live [ci], **NEW** rust:absent_tmux_server_is_empty_but_broken_tmux_runtime_is_an_error |
| `06f0d91`, `84822d5`, `2c0f977` v0.1.28 | Inside a managed pane, external discovery listed managed sessions as external (shared `$0`), with duplicate rows and broken remote attach | inherited `TMUX`/`TMUX_PANE` redirected default-server tmux calls | py:tmux_discovery_live [ci], **NEW** rust:inherited_tmux_environment_never_redirects_default_server_discovery |
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
| `b5322e5` v0.1.7 | The pane command changed between confirmation and stop | the tmux command was not rechecked at stop time | py:stop_shell_live [ci] |
| `4f8cb9b` | Native-command fixtures killed process groups they did not own | cleanup was too broad | py:native_command_pty [ci] (fixture hygiene) |
| `abb4a39` v0.1.3 | One-shot commands left job groups behind, or broke Fish/Bash/Zsh wrappers | foreground process-group control | py:native_command_pty [ci] |
| `6188285` | The enrollment installer left writer/reader threads blocked | an exited child's inherited pipes were held open | rust:exited_installer_cannot_leave_writer_or_reader_blocked |
| `9238073` | Helper exit with inherited stderr hung cleanup | a descendant kept stderr open | rust:helper_exit_with_inherited_stderr_is_bounded_and_owned_group_is_reaped |
| VALIDATION / `f91242a` | A stalled relay-version or capability check froze adoption | wrapper descendants kept pipes open | rust:relay_version_preflight_is_bounded, rust:relay_version_does_not_wait_for_wrapper_descendants, rust:relay_version_bounds_output_and_requires_success |
| misc | Bounded subprocesses | | rust:bounded_process_timeout_cleans_descendant_and_reader_threads, rust:stalled_terminfo_is_bounded_and_reaped, rust:bash_monitor_mode_is_disabled_for_noninteractive_probe_work |
| `8c110ec` v0.1.77 | A terminal that ended during a move left an intent record | the record was written before the move succeeded | py:adopt_stress_live [local] (needs a ptrace sandbox) |
| v0.2.2 | 64 `attach-guard-*` files piled up in the state dir | the guard lived until the attach ended, and a process that dies without unwinding (the closed-terminal watchdog's `_exit`, SIGHUP over ssh) never removed it; no sweep | rust:checked_attach_requires_guard_success_and_preserves_external_config ("a passed guard must not outlive its check") **NEW**, rust:startup_sweep_removes_only_stale_unowned_attach_leftovers **NEW** |

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
| v0.2.2 | cx would not open in half terminals (80x48, 95x50): `hub.rs:43 min > max. min = 7, max = 6` | the tile height's floor (7 at h ≥ 36) passed its cap (6 below 100 columns) inside `clamp`; the phone hub indexed an empty device list | rust:every_view_renders_across_sizes **NEW** (suite), rust:every_view_renders_at_every_size [ignored, the full 20x5..250x80 matrix] **NEW** |
| v0.2.2 | Hub thumbnails stayed blank (title, `idle 0s`, folder, empty rows) | only the selected session was ever captured; a tile drew a frame remembered from an earlier selection | py:hub_thumbnails_pty [ci] **NEW** (red on v0.2.1) |
| v0.2.2 | A never-seen device read `✕ 20770d` | its age was counted from `fetched == 0` (the epoch) | rust:a_never_seen_device_shows_no_age **NEW** |
| v0.2.2 | Chrome painted cell backgrounds over transparent terminals | tints, reversed keycaps and surfaces | rust:chrome_paints_no_background **NEW**, rust:ascii_capability_renders_no_unicode_and_keeps_focus_cues |
| v0.2.2 | The arrows did not move the find/palette selection | the palette drew the hub's selection (`caps.selected`), never `palette_selected`, and did not scroll to it; Ctrl+N/P were unbound in inputs | rust:palette_arrows_move_the_drawn_selection_and_the_cursor **NEW**, py:grid_keys_pty [ci] **NEW** (red on v0.2.1) |
| v0.2.2 | The session grid could not be walked (only a line: Left/Right, Tab) | moves were indexes into the attention order; nothing knew where tiles and nodes were drawn | rust:the_hub_tiles_are_a_grid_the_arrows_walk, rust:the_map_arrows_move_between_nodes_by_place, rust:a_device_grid_walks_with_arrows_and_taps_focus **NEW**, py:grid_keys_pty **NEW** |
| v0.2.2 | The command panel panicked under 20 columns (`rendering.rs:807 min > max. min = 16`); choosers and the context menu drew outside tiny frames | an inverted `clamp`; rects past the frame | rust:every_overlay_renders_across_sizes **NEW** (every App overlay from 1x1), rust:every_overlay_renders_at_every_size [ignored, 1x1..250x80] **NEW** |
| v0.2.2 | A device's LINKS were no zone: Tab stepped tiles, Up from the top row jumped along the order; Esc, a key, Esc read as a double Esc | the device view had one zone; the Esc pair survived other keys | py:map_grid_pty **NEW** (map by place, device grid in 2D, LINKS zone, tap focus; red on v0.2.1) |
| v0.2.2 | A fresh shell on a loaded host could read `recent: env`; shell_labels_live flaked in the full run | panes start as `exec env … shell`, and a listing in that window recorded `env`; the test typed before Fish drew its prompt | rust:shell_labels_never_include_arguments_or_terminal_controls (`env`), py:shell_labels_live (waits for the prompt) |
| v0.2.2 | The hub map's two halves looked different; the arrows jumped along an order when nothing lay that way; an idle hub redrew every second | each half laid out by its own rules; Left/Right fell back to the hub order; the vitals strip counted seconds | rust:a_symmetric_network_draws_as_a_mirror_image, rust:moves_reverse_reach_everything_and_never_invert, rust:the_arrows_reach_every_device **NEW**, py:hub_motion_pty **NEW**, perf_harness idle.hub = 0 bytes |
| v0.2.2 | The production hub drew links as plain ssh or a jump, with no vitals, meshes, lan or rtt (only the lab fixtures had them) | the view-model carried no mesh membership, link samples or device metrics | rust:cpu_comes_from_successive_samples_and_rtt_from_round_trips, rust:only_helpers_with_metrics_are_sampled_once_at_a_time, rust:members_map_to_devices_by_address_then_name, rust:lan_evidence_is_a_neighbour_or_a_shared_subnet **NEW**, py:hub_matrix [ssh] (real viewer and helpers, 8 configurations x 6 sizes x 4 tiers against the lab) **NEW**, perf_harness idle.hub = 0 bytes with metrics on (frozen CX_METRICS_ROOT) |
| v0.2.2 | Key pills could paint outside their rounded caps | (new: the only sanctioned background) | rust:pills_are_the_only_background_and_always_capped **NEW** |
| v0.2.2 | Left closed the new-session chooser (its choices sit in a row) and the agent step | an explicit "Left is back" rule in `dialog_key` dismissed the popup | rust:arrows_move_in_popups_and_only_esc_closes_them **NEW**, py:chooser_arrows_pty **NEW** (red on v0.2.1) |
| v0.2.2 | The hub key `` ` `` has no key of its own on a 65% keyboard | (user feedback) | rust:hub_letters_open_devices_and_shift_h_returns_from_anywhere, py:grid_keys_pty (Shift+H from Files) |
| v0.2.2 | After an update replaced `~/.local/bin/cx`, a running viewer could not attach local sessions and a running helper refused typed input | `current_exe()` reads `<path> (deleted)` once the file is renamed over; cx handed that path to tmux's attach-check predicate (and askpass, transfers, native commands, containers, enrollment) | rust:a_replaced_binary_resolves_to_the_new_file_at_its_path **NEW**, py:swap_attach_pty [ci] and py:swap_attach_pty@ssh **NEW** (red on f011523) |
| v0.2.2 | bash/sh users started from a terminal were never offered Claude/Codex locally | the `SHELL -l -i -c 'claude --version'` probe ran in its own group of cx's terminal; interactive bash took that terminal for job control and was stopped by SIGTTOU until the 3 s timeout | py:provider_probe_tty [ci] **NEW** (red on f011523: bash 6.0 s, nothing offered) |
| unreleased (own040-launch; from launch030 + filesbinds032) | Starting an agent took the full chooser every time (about 5 keys for a YOLO agent again), its folder was always the device's home, and bare `c x n` on the hub worked only with one device (a device could hold the letter `c`, `x` or `n`) | no remembered mode; no notion of the folder `cx` was started in; letters only withheld `e` | **NEW** rust:hub_c_x_n_start_where_cx_was_started_on_this_device_in_the_last_mode, rust:device_keys_start_in_here_locally_and_in_the_last_files_folder_remotely, rust:c_and_x_reuse_each_agents_last_mode_per_device_and_viewers_share_it, rust:remembered_yolo_on_an_old_remote_helper_starts_nothing, rust:remembered_yolo_with_stale_capabilities_checks_then_starts, rust:a_waiting_yolo_launch_is_never_dropped_silently_nor_started_after_moving_on, rust:files_bang_then_c_x_n_starts_in_the_folder_in_the_last_mode, rust:hub_shows_where_bare_launch_keys_start_at_every_size, rust:ghjkl_only_past_17_devices_and_never_e_c_x_n, rust:launch_modes_are_per_device_and_agent_and_merge, py:launch_keys_pty [ci] (`cx` in a folder then `c`; stand-in argv and folder for every path) |
| unreleased (050-dead) | Enter on a terminal that had ended said "Attachment failed: Session changed or ended" and threw the user to the hub; a closed outside-tmux terminal stayed offered for up to 30 s (forever from the startup cache), and Enter on it failed the move without removing it | the attach's return always ran `returned_from` (the hub), success or not; outside terminals were refreshed only by the 30 s fleet refresh, restored from the disk cache as if confirmed, and a failed AdoptSession left its row | **NEW** rust:refused_attach_stays_on_the_device_and_removes_the_tile, rust:refused_attach_from_the_hub_stays_on_the_hub, rust:other_attach_failures_report_without_navigating, rust:detach_still_returns_to_the_hub_with_the_tile_selected, rust:failed_adopt_of_a_closed_terminal_removes_it_and_rediscovers, rust:cached_outside_terminals_stay_hidden_until_confirmed, rust:the_open_device_polls_its_outside_terminals, rust:enter_on_an_unreachable_cached_tile_only_says_why, rust:a_refusal_without_the_gone_status_keeps_the_tile, rust:remote_checked_attach_maps_only_the_gone_status_to_gone, rust:checked_attach_requires_guard_success_and_preserves_external_config (listed session's failure is not Gone; unlisted is), rust:an_outside_scan_from_before_a_move_never_resurrects_the_moved_terminal, py:dead_terminals_pty [ci] (red on v0.4.0: the closed outside tile stayed, and Enter on the killed shell landed on `cx › sessions`) |

| v0.5.0 (050-agents) | (Guard for new keys) `a` and `o` start Antigravity and opencode, so they must never be session slots or device letters; the hub's attach hint must name the first tile's real key (it said `a`) | slots and letters were static pools; `hub.rs` hard-coded `a attach` | **NEW** rust:a_session_keeps_its_key_until_it_ends_and_new_ones_take_the_first_free (`s d g q u i y z v m 1-9`, HOME `s d g`), rust:ghjkl_only_past_15_devices_and_never_e_c_x_a_o_n (1–25 devices), rust:no_static_binding_takes_a_device_letter_on_the_hub, rust:every_agent_has_its_new_and_resume_keys_on_the_hub_and_device_screens, rust:bang_then_an_agent_key_launches_that_agent, rust:agy_and_opencode_are_offered_only_where_info_lists_them, py:launch_keys_pty [ci] (`a A o O`, `t A`, `t o`, `!` `a`), py:cli_launch_pty [ci] (`cx agy`, `cx antigravity --yolo -r`, `cx opencode`, `-p` refused) |

## 6. SSH routes, neighbours and transport

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `2dc00fe` | Jump chains could loop or grow without bound | | rust:jump_routes_are_bounded_validated_and_acyclic |
| `de1d0a5` | Native attach and helper calls built different SSH routes | duplicated SSH construction | py:jump_routes_cli [ci] (transport-master: keep a single constructor) |
| `c2e6a79` v0.1.13 | Background jump-host auth prompts; enrollment retargeted a device on an overlapping network | missing BatchMode on routes from personal SSH config; no overlap check | py:jump_routes_cli [ci], py:access_routes_cli [ci] |
| `6188285` v0.1.16 | A queued SSH check outlived its network snapshot | stale queue entries | rust:queued_neighbor_check_does_not_outlive_snapshot |
| `1016401` v0.1.16 | Automatic port checks were unbounded and caused redraws | | rust:network_auto_ssh_is_bounded_cached_and_scoped_without_redraw_work, rust:network_auto_ssh_ignores_stale_unnumeric_and_missing_interface |
| v0.1.13–15 | Addresses sorted as strings; stale ports stayed "open"; a refused connection showed as unknown | | rust:network_numeric_order_stale_port_and_linklocal_enrollment, rust:network_neighbors_sorted_and_unknown_is_honest |
| `17aade9` v0.1.32 | Robot neighbour names were missing; manual Add lost the selected gateway | the resolver output was not validated; the observer was not preserved | rust:resolver_output_is_only_a_valid_name_for_the_requested_address, rust:named_neighbors_are_visible_and_manual_add_preserves_selected_observer, rust:network_device_has_direct_terminal_without_creating_a_tmux_session, py:login_terminal_pty [ci] |
| `bb33fdd` | Enrollment broke when the robot's DHCP lease changed | a DHCP address was pinned instead of the `.local` name | rust:observer_names_stay_names_but_ambiguous_devices_are_rejected (replaced the `17aade9` numeric-IPv4 test) |
| `536853a` v0.1.12 | A closed metadata channel failed listing; raw SSH stderr was shown | there was no single reconnect; errors were unclassified | rust:disconnected_observations_retry_once_but_mutations_never_replay, rust:ssh_failures_are_actionable_and_never_expose_stderr, rust:actual_helper_stderr_is_bounded_and_classified, py:metadata_reconnect_cli [ci] |
| `9238073` | The cached-session remedy was clipped at small widths | | rust:failed_cached_session_keeps_refresh_visible_at_small_widths |
| `d381c3e` v0.1.34 | Robot authentication exceeded the helper deadline | GSSAPI was tried before public keys | UNGUARDED: nothing asserts `PreferredAuthentications=publickey,...` first (src/store.rs, transport-master) |
| `d381c3e` v0.1.34 | Shell attach failed at once on a robot lacking the viewer's terminfo | vendor TERM | py:attach_terminal_pty [ci] |
| VALIDATION v0.1.13 | Identity probe failed on Arch | `hostname` missing; now `uname -n` | UNGUARDED |
| VALIDATION v0.1.7 | Network observation failed on Rocky 8 | `ip` lives in `/usr/sbin`, outside PATH | UNGUARDED (`src/network.rs` candidate list) |
| `8966b68`, `8b20780` v0.1.72 | Pairing claimed bidirectional connectivity from local posture | | py:peer_pairing_cli [ci] |
| `6cdf1b5` v0.1.76 | Managed SSH directories were group-writable under umask 0002 | inherited umask | rust:recovers_only_empty_legacy_root_beneath_private_parent, py:peer_mesh_cli [ci] |
| `260e7da` v0.1.74 | The static build failed on the `renameat2` symbol | libc wrapper missing in musl | build-only (portable.yml musl build) |
| `5fe00ab` v0.1.75 | A redundant background helper update ran during enrollment | | py:enrollment_tooling_cli [ci] |
| `aa51d80` | Shared-LAN rules broke when the boat changed ports; docker0 was masqueraded | rules named interfaces | rust:host_nat_does_not_claim_internet_without_robot_gateway_dns_and_verification (asserts `oifname != "docker0"`), py:sharing_lan_namespace [local] |
| `5bb18e3` | Sharing chose its upstream from stale measurements | | rust:upstream_selection_prefers_throughput_then_latency_and_rejects_stale |
| `f3fecb5` | Mesh teardown used an empty account | the account came only from discovery | py:mesh_util [ci] |
| v0.2.1 review T3 | release.yml listed the `portable` job twice, so Actions would reject the workflow and a tag would publish nothing | a hand edit; nothing parsed the workflows | py:workflow_keys [ci] |
| v0.2.1 review F15 | files_bench showed a 100k sibling folder at ~85 ms in some runs, and every first-rows number ~40 ms high | the bench's screen readers rebuilt pyte's display once per line (~40 ms a check), twice when a frame arrived in two reads; cx drew the rows 37-42 ms after Enter in every run | py:files_bench --self-test [ci] |
| revamp transport-master review | A terminal could become the shared SSH master, so closing its pane ended every channel on the route; a stalled master handshake held requests for up to 25 s; cleanup matched a 20-bit tag; a missing managed trust file failed command construction; orphans closed mesh-endpoint masters; a silently dead master kept answering `-O check` | `ControlMaster=auto` on terminals; synchronous master start under the slot lock; tag-prefix matching; `File::open(known_hosts)?`; inventory-only ownership; no liveness probe | py:ssh_multiplex [ci] **NEW**, py:peer_managed_ssh [ci] (owner record, mesh master not stray, missing trust file), rust:closing_masters_matches_the_recorded_target_exactly, rust:managed_trust_reads_missing_files_as_empty, rust:masters_are_stray_only_when_idle_and_unreusable |

## 7. Session identity, labels and runtime inventory

| Commit | Symptom | Root cause | Guard |
|---|---|---|---|
| `ec586d6` v0.1.7 | An agent's folder came from the tmux shell cwd | it used the shell cwd instead of the agent's workspace | rust:codex_workspace_argument_ignores_prompt_and_config_values, rust:provider_directory_rejects_pid_reuse, py:session_directory_live [local] |
| `c9ed1b8` v0.1.7 | Generated titles never refreshed | | rust:generated_session_names_follow_provider_workspace_and_keep_custom_names |
| v0.1.19 | Renamed sessions reordered the list | | rust:changing_session_labels_preserves_row_order_and_selection |
| `64c41ab` v0.1.22 | A blank saved name gave an empty label | the blank name skipped native lookup | rust:generated_session_names_follow_provider_workspace_and_keep_custom_names |
| `7bec0b9` v0.1.27 | Fresh shells showed a hash or directory label | | py:shell_labels_live [ci], rust:shell_labels_never_include_arguments_or_terminal_controls |
| `12b1069` | Multicall executables were mislabelled | | py:manual_agents_live [ci] |
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
| v0.3.0 | Fixture viewers and probes (capture_matrix, perf_harness, files_bench, every PTY fixture) ran adoption discovery over the host's real `/proc` | isolated HOME and TMUX_TMPDIR do not hide processes | **NEW** py:probe_isolation [ci] (on the host a debug build lists no terminal, the outside one included; under tests/isolate.sh only the fixture's own); every fixture but the docker and `hostids` ones, and every `pty_lib.isolate()` harness, runs under tests/isolate.sh, which always makes new namespaces; `pty_lib.Pty` refuses to start outside one; a debug build (every fixture's BIN, peer_mesh_cli's viewer included) discovers terminals only under tests/isolate.sh (a non-initial pid namespace plus its /dev/shm marker, so not in a container's own namespace either, which probe_isolation checks); every standalone harness that starts a viewer (validation_review, capture_ui and the legacy PTY scripts too) calls `isolation.isolate()` first; rust `sessions_stay_managed_only_while_outside_terminals_use_their_own_operation` re-runs itself under tests/isolate.sh |
| v0.4.0 (own040) | Moved terminals never showed up ("taking unusually long"), the old window kept its last output, and the moved agent died minutes later; `⌫` on a moved session said "identity changed" | success read the relay's `/proc/<pid>/fd`, hidden from the user once a setcap or sudo relay is non-dumpable, so the record kept its pre-move identity and the holder was never closed (and SIGHUP alone is ignored by foot, whose later close signals the moved group HUP/TERM/KILL) | **NEW** py:adopt_setcap_live [local] (setcap relay as an unprivileged namespace user, foot-like emulator, stale-record healing, StopExternal), rust:a_move_is_seen_from_the_holder_and_leader_alone |
| v0.4.0 (own040) | `⌫` on the hub and in peek asked invisibly (the next key cancelled it); remote stops needed three presses; split sessions, outside-tmux agents and setsid'd children could not be stopped | the stop focus was set only for the device view; a stale capability check refused the first press; single-pane shape guard; no external stop; kill-session only hangs up panes | **NEW** rust:hub_and_peek_show_the_stop_question_and_a_second_backspace_stops, rust:one_double_backspace_stops_a_remote_session_with_a_stale_check, rust:backspace_stops_an_outside_agent_through_stop_external, rust:reaping_ends_only_snapshot_processes_that_outlived_their_session, py:stop_shell_live (split session refused without `all_panes`, stopped whole with it; setsid escapee), rust:only_a_known_emulator_holder_is_killed_others_get_sighup, rust:a_move_is_seen_from_holder_and_leader_and_an_exited_holder_needs_the_terminal, rust:stop_external_refuses_a_changed_boot_or_an_unlisted_terminal, py:adopt_setcap_live (`cx adopt` by pid, tty and list; `cx kill` of an outside agent) |
| v0.5.0 (050-agents) | (Guard for new agents) agy's updater and daemons (`--bg-updater`, `remote-control`, `acp`, …), `opencode serve` and opencode's other commands must not count as agent sessions; an auto-updated agy (`agy (deleted)`) and opencode's npm wrapper and native child must (as one agent) | `running_provider` reads exe/script and the table's `not_agent_args` (first argument only) | **NEW** rust:agy_and_opencode_processes_are_told_from_their_helpers, rust:the_opencode_wrapper_and_its_child_are_one_agent |
| v0.5.0 (050-agents review) | Bare `a`/`o` (or a device letter then them) started agy/opencode on a device whose fresh Info said it was not installed: a 30 s "launch failed (127)" pane, or "unsupported" from an old helper | `quick_launch` never looked at the device's capabilities | **NEW** rust:an_agent_key_for_a_missing_agent_starts_nothing_while_info_is_fresh (red before: "agy launched though Info lacks it") |
| v0.5.0 (050-agents review) | (Guard) a shell StopSession from an old viewer on a pane running a node-run agent (npm opencode, claude, codex: tmux's pane command reads `node`) must be refused | `no_agent_command_guard` matches exe names only; `reject_provider_descendants` is script-aware and covers it | **NEW** rust:a_shell_stop_refuses_a_node_run_agent_in_the_pane |
| v0.5.0 (050-agents review) | An agy/opencode auto-update made the whole `providers.json` stale, so the next Claude launch probed every agent in login shells again | `ProviderProbe::current` checked every binary | **NEW** rust:an_updated_agent_makes_only_its_own_probe_stale (`current_for` + `with_agent`: only the changed agent is probed again and merged) |
| v0.5.0 (050-agents review) | (Guard) stopping opencode must stop its own server; Ctrl+\ must detach while agy has the kitty keyboard protocol on | `--standalone` keeps `serve` in the session's tree, which the stop reaps; Ctrl+\ is a tmux root binding | **NEW** py:cli_launch_pty [ci] (a HUP-ignoring `opencode serve` child is gone after `cx kill`), py:agent_wheel_pty [ci] (CSI >1u / =1u, then 0x1c and CSI 92;5u detach, the app receives neither) |
| v0.5.0 (050-window) | A moved terminal's window disappeared from the desktop (the frozen holder was killed with nothing in its place) | the takeover only closed the source window | **NEW** py:adopt_setcap_live [local] (recorder emulator: `cx attach <id>`, leader cwd, whitelisted env, own session, systemd-run scope with a reachable bus, PATH fallback, missing emulator still closes, `emacs` holder never reopened or SIGKILLed, the holder already gone when the new window starts, outside `agy`/`opencode` terminals moved and replaced), rust:a_replacement_window_runs_attach_in_each_emulators_command_form, rust:a_replacement_window_gets_only_display_variables_from_its_holder, rust:an_updated_emulator_is_found_at_its_path_or_on_path, rust:a_replacement_window_starts_detached_with_only_its_environment, rust:a_move_reports_what_became_of_its_source_window |

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
| `36cd089`, `7fba202` v0.1.3–4 | Ctrl+C/Ctrl+Z and builtins misbehaved in POSIX shell profiles | | py:native_command_pty [ci] (also CX_TEST_SHELL=/bin/sh) |
| `20a3737` v0.1.85 | Stale "Connection closed" text showed on session handoff | LeaveAlternateScreen restored the old primary buffer | py:terminal_startup_loader_pty [ci] (raw `\e[?1049l\e[2J`) |
| `2acff83` v0.1.11 | OSC 52 copies never reached the viewer | | py:clipboard_pty [ci] |
| v0.3.0-pre (hotfix030) | Stuck in Files on real terminals: keys typed into a stray `run:` prompt, no visible way out | a startup-probe reply arriving after its 100 ms limit reached the event loop as keys (`ESC _ G i=31;OK ESC \\` read as Alt+_, G, i, =, 3, 1, `;` (yazi's shell prompt), O, K); the old guard only knew OSC 11 and swallowed every key after an Alt+] | **NEW** py:terminal_replies_pty (kitty/ghostty/foot replies prompt, late, split, inside tmux), rust:late_probe_replies_are_dropped_only_when_complete, rust:yazi_esc_clears_first_then_leaves_files_and_ctrl_s_only_cancels_find |
| v0.3.0-pre (hotfix030) | Hub links drew v0.2.2's continuous braille curves on a direct, ungrouped fleet | key links stepped every other raster dot (a solid run on shallow curves) and a lit link turned solid; WARP members reached by overlay address were never grouped without a cx-deployed mesh | **NEW** rust:links_are_evenly_dotted_never_solid_runs, rust:a_link_dots_evenly_along_any_slope, rust:overlay_addresses_group_devices_without_mesh_state, py:hub_matrix `fleet-5` |
| v0.3.3 (hotfix033) | A remote device whose mesh name stopped resolving showed no sessions and blank previews, silently; its outside-tmux agent terminals never appeared remotely | only a failed listing marked a device down (screen reads, vitals and Info failures were dropped) and DNS/refused/timeout were one message; no fallback to addresses seen working; external discovery waited for the next 30 s refresh after Info | **NEW** rust:unreachable_devices_say_why_on_every_tile, rust:remote_outside_terminals_are_marked_tiles_not_managed_sessions, rust:fallback_addresses_pin_the_original_host_key, rust:own_addresses_are_the_overlay_and_a_real_lan, rust:ssh_failures_are_actionable_and_never_expose_stderr, py:address_fallback_pty [ssh] (loopback sshd: fallback via recorded LAN address with HostKeyAlias, different key refused, reasons within the timeout, remote outside tile) |
| v0.3.5 (hotfix035) | A connection blip threw the user out of an attached remote terminal back to the hub, its tile hovered | every nonzero exit of the attach's ssh (255: connection lost) ended the attach like a detach; terminals and their shared master gave up after 10 s of silence (ServerAlive 5 × 2) | **NEW** py:reconnect_pty [ssh] (loopback sshd: killed connections with and without the master reattach the same pane, banner, typed input; Esc while sshd hangs returns to the focused tile; a session ended meanwhile is reported; Ctrl+\ detach stays immediate), rust:terminals_and_their_master_ride_out_blips, rust:reconnect_backoff_and_banner |
| v0.3.0 | Files `h`, the hub peek and Enter -> attached were slower than in v0.2.2 | each frame's bytes (an absolute cursor move per gap, spaces for every erased cell); each Files frame's work (two passes over a 100k listing, the hub view model and a 512-name row map); the 2 s vitals shared the interactive lane with select and attach | **NEW** rust:encoded_frames_show_exactly_the_buffer_in_fewer_bytes, rust:interactive_work_never_shares_a_channel_with_bulk_work, files_bench + perf_harness A/B against v0.2.2 |
| v0.4.0 (own040) | Opening a new session was slow: `c` -> the agent's first frame took the agent's startup plus ~430 ms (1.73 s for a 1 s agent behind a 0.3 s login rc); the chooser offered only Shell until a recheck answered | Create probed `claude/codex --version` (and `codex --help`) in login shells before starting, then held its reply until the pane showed text (80 ms polls, up to 5 s); Info probed one provider after another; the chooser dropped a device's agents 60 s after its last Info; Create queued behind refreshes and probes on the device lane | **NEW** perf_harness create_agent (A/B: 1728 -> 1323 ms, attached 59 ms), py:terminal_startup_readiness_live [ci] (replies at once; a failed launch keeps its pane with its status for the grace; a provider gone since the probe; an end before the reply is reported) and py:terminal_startup_loader_pty [ci] (native attach while the shell starts), both red on v0.3.5; rust:launches_trust_a_recent_probe_of_the_same_shell_only (a recorded `--no-daemon` holds only for the unchanged codex file; py:codex_process_mode_live, py:mouse_scrollback_pty unchanged), rust:tmux_list_arguments_keep_a_trailing_semicolon, rust:starting_or_stopping_a_session_runs_next_on_its_lane, rust:an_old_info_still_offers_agents_without_waiting_for_a_recheck |

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

- **Heavy (screen text, labels and positions):** `session_chooser_pty` ("[n] Shell", bold selection), `list_wheel_pty`,
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
- **Not coupled (tmux, helper or CLI only):** `mouse_scrollback_pty`, `codex_wheel_pty`, `codex_wheel_threads_pty`, `agent_wheel_pty`,
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
personal tmux server, even when the script runs inside a cx pane. Every fixture except the docker and `hostids` ones runs under `tests/isolate.sh`
(private user, pid, mount and cgroup namespaces): it sees only its own processes, never a real terminal, and no user bus; any real `agy` or `opencode` on the caller's PATH is covered by an empty, non-executable file there, so only a fixture's stand-ins can run. The others run
the debug build, which discovers no terminal outside a private pid namespace. Fixtures with an unmet requirement (pyte, pillow, fish, poppler,
sshd, docker, node with `node:sqlite`) are reported as SKIP, never as PASS; CI fails on any SKIP. `mesh_worker.mjs` runs with node.
