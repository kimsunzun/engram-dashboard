# Step 22 (hook components) — codebase facts (Explore sweep 2026-11-24, condensed by main)

Paths: AL = crates/engram-dashboard-asset-library/src, AG = crates/engram-dashboard-agent/src.

## Skill flow today (template for a hook kind)
- Scan: AL/catalog.rs FileKind::of (SKILL.md directly in a folder below a mount top -> SkillFile(folder)); ScanResult::build emits ItemKind::Skill (ItemContent::Skill {name, description, file_name, folder, text}) + ItemKind::SkillFile; ItemKind::listed, apply_scan, skill_file_key.
- Preset format: AL/format/read.rs enum ListName {Presets, Instructions, Skills, Settings, Folders}, ListName::ALL, key(), takes_tags() (Instructions, Skills take backend: tag); PresetDoc lists; parse_preset item_names. Writer AL/format/write.rs PresetLists + per-list table.
- Edit: AL/edit.rs list_of(kind) (ItemKind::Skill -> ListName::Skills), doc_list, draft_lists, set_backend, view_parts. Relink AL/relink.rs uses list_of.
- Refs: AL/catalog.rs preset_refs feeds used_by/refs_to; AL/refs.rs closure/referrers_of/reference_count; AL/resolve.rs refs() tag filter.
- Resolve: AL/resolve.rs own_lists, fn skill (look, wrong_kind, agent.loads.skills false -> omitted_skills, check_skill via port skill_format, accepts_skill_name), conflicts (SKILL_NAME_CONFLICT), omitted_notice, finish.
- Launch: AL/launch.rs launch_check (LaunchPlan.skills, extra_skill_roots, same_names), overlay_read (stamp), prepare_in, volatile_dir <launch_root>\<id>-<epoch>, release_dir, sweep_launch_dir at boot.
- Agent types AG/launch.rs: SkillRef, LaunchPlan, LaunchKit, LaunchArtifacts {skill_roots, settings_file, add_dirs, env, config_args}; SKILL_LINKS_PARENT="engram", SKILL_LINKS_DIR="skills"; LaunchFs (write_file/link_dir), place_in_volatile (all writes confined to the volatile folder); notice_text loop.
- Other skill-aware places: AL/lib.rs folders()/row(); AL/host.rs preset_reorder (PresetSection->ListName), preset_view, preset_draft, library_kind; AL/gate.rs item_problems; AL/create.rs; AL/builtin.rs; AL/ops.rs (reclassify, refuse_into_itself, move_in, merge, follow_moves, lone_skill_file, skill_files, SKILL_UNCONFIRMED); AG/commands.rs LibraryKind; AG/library_commands.rs PresetSection (ALL, word, parse), preset.save/check skills args; agent tests/command_declarations.rs (five section words); prompts/engram-help.md (kind table, example preset, backend tag rule, --kinds, preset.reorder/save/check, skill-unconfirmed text). Frontend has no skill code yet.

## Claude today
- Plugin dir (ADR-0316): AG/backend/claude/launch.rs materialize + skill_links; only when kit.skills non-empty; <vol>\engram\skills\<name> junctions; --plugin-dir <vol>\engram via LaunchArtifacts.skill_roots; skills appear as engram:<name>. No .claude-plugin/plugin.json written. docs/research/skill-md-recognition-claude-codex-2026-10-20.md row 7: no manifest needed when skills/ exists; components read without following symlinks; codex also treats .claude-plugin/plugin.json as a plugin marker (renames skills).
- place_in_volatile allows <vol>\engram\hooks\hooks.json structurally. Unmeasured: manifest-less plugin picks up hooks/hooks.json; ${CLAUDE_PLUGIN_ROOT} script paths given junctions are not followed for components.
- Merged settings (ADR-0328): AG/backend/claude/settings.rs merge/merge_two/without_repeats — later wins, objects recursive, lists concatenated + de-duplicated; written <vol>\settings.json, one --settings. A hooks key in a settings preset already passes through untouched (ADR-0359: no schema check); script paths stay as written (outside the library today). HOOK_KEYS/UnsureWhy::HookKeys only for subagent front matter preview. Claude does not load settings hooks from an --add-dir folder (PRD/TRD risk table).

## Codex today
- JSON: AG/backend/codex/launch.rs json(): junctions <vol>/engram/skills/<name>, skills/extraRoots/set (fail closed); developerInstructions on thread/start|resume.
- Terminal: loads() skills false; codex-terminal-omitted; only preset -c lines + developer_instructions.
- -c lines: preset_config_args -> cmdline::config_args(raw) (raw = array of "key=value"); split_config_line, text_is_cli_safe guard, even quotes, cmdline::fits 8191.
- History: ADR-0210 (superseded) registered -c hooks.SessionStart=[...]; ADR-0216 removed it — codex hashes the hook command and needs human trust; untrusted hook skipped silently in 0.129+, terminal approval screen blocks startup; trust broke each release because of a versioned exe path. AG/backend/codex/mod.rs keeps tests asserting no hooks.SessionStart override (hook_override_value). Implication: a volatile script path (<vol>\... changes per id-epoch) would need re-approval every launch; a stable path trusts once.
- ADR-0353 fail-closed-before-write pattern applies to any hook route.

## Port and ADR-0367
- AG/backend/settings_check.rs trait SettingsCheck: backend, loads(mode)->ModeLoads {instructions, skills}, accepts_skill_name, folder_entry_blocks, working_folder_blocks, notice_codes {omitted, folder_omitted, same_name}, folder_listing_at_launch, folder_load_set, skill_format, command_line_args, command_line_fits, instructions_append, native_skill_roots, root_skills. Impl: AG/backend/claude/settings.rs, AG/backend/codex/settings.rs, fakes AL/backends.rs (fake third backend fakebe).
- ADR-0367: asset-library must not branch on backend words; per-backend differences are port answers -> a hook kind needs e.g. ModeLoads.hooks, a hook-format answer, a notice code for modes that drop hooks; layout on disk only in each backend's materialize; if a hook folder holds per-backend definitions, selection = port answer or backend tag. TRD §8-3 gate: rg -i '\b(claude|codex)\b' no branch hits in asset-library.
- ADR-0359 decision 4: no hook schema check; check only what assembly can break.

## Mentions
- No hook library/preset concept in code. trd.md step 22 item (open items). ADR-0210/0216 (codex hooks), 0257/0262 (killing claude hook processes after interrupt), tracking T-41 (Esc during first-turn hook). docs/research/agent-preset-components-survey-2026-10-11.md (plugin bundles carry hooks in both CLIs; --plugin-dir session-only); docs/process/S21-chat-ux/trd-t40.md (claude hook runner spawn).
