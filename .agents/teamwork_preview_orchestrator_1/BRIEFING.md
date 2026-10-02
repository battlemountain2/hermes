# BRIEFING — 2026-10-01T03:02:10Z

## Mission
Orchestrate and execute Phase 1 of Hermes preview and UI overhaul (R1 GIS GeoTIFF, R2 Pooled Sandbox Worker, R3 Rich Format Previews, R4 Status Bar Completion) with full test coverage and verified integrity.

## 🔒 My Identity
- Archetype: teamwork_preview_orchestrator
- Roles: orchestrator, user_liaison, human_reporter, successor
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1
- Original parent: sentinel
- Original parent conversation ID: de5db9f9-e8e0-4efe-98aa-deaf92fdf84f

## 🔒 My Workflow
- **Pattern**: Project
- **Scope document**: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/PROJECT.md
1. **Decompose**: Survey codebase via 3 Explorers, create Feature Inventory in PROJECT.md, define Milestones (M1-M4, E2E Testing, M5 Final Integration/E2E Pass & Hardening)
2. **Dispatch & Execute** (pick ONE):
   - **Delegate (sub-orchestrator)**: Delegate milestones and test track to sub-orchestrators
3. **On failure** (in this order):
   - Retry: nudge stuck agent or re-send task
   - Replace: spawn fresh agent with partial progress
   - Skip: proceed without (only if non-critical)
   - Redistribute: split stuck agent's remaining work
   - Redesign: re-partition decomposition
   - Escalate: report to parent (sub-orchestrators only, last resort)
4. **Succession**: at 16 spawns, write handoff.md, spawn successor
- **Work items**:
  1. Survey phase [in-progress]
  2. Decomposition & PROJECT.md [pending]
  3. E2E Test Track [pending]
  4. Milestone Execution [pending]
  5. Final E2E Pass & Hardening [pending]
- **Current phase**: 0 (Survey)
- **Current focus**: Survey codebase via 3 parallel explorers

## 🔒 Key Constraints
- NEVER write, modify, or create source code files directly.
- NEVER run build/test commands yourself — require workers to do so.
- NEVER investigate or explore the problem at the code level — dispatch Explorers for technical investigation.
- You MAY use file-editing tools ONLY for metadata/state files (.md) in your .agents/ folder.
- Never reuse a subagent after it has delivered its handoff — always spawn fresh.
- Maintain all 181 existing tests passing without regression.
- Forensic Auditor is a BINARY VETO — violation means failure, no exceptions.

## Current Parent
- Conversation ID: de5db9f9-e8e0-4efe-98aa-deaf92fdf84f
- Updated: not yet

## Key Decisions Made
- Project Orchestration Pattern selected.
- Scope document: PROJECT.md in orchestrator folder.
- Survey phase initiated with 3 parallel explorers to inspect codebase architecture.

## Team Roster
| Agent | Type | Work Item | Status | Conv ID |
|-------|------|-----------|--------|---------|
| explorer_survey_1 | teamwork_preview_explorer | Survey GIS GeoTIFF Architecture | completed | 50c48977-6d8b-4fa6-a56d-d41d9cc03a36 |
| explorer_survey_2 | teamwork_preview_explorer | Survey Sandbox Worker & Guardrails | completed | f24215ad-6168-4b51-8c0d-48f7ceb53d8f |
| explorer_survey_3 | teamwork_preview_explorer | Survey Rich Previews, Status Bar & Baseline | completed | 11363f58-2fcc-45a8-8cbb-c4155337d666 |
| test_writer_e2e_1 | teamwork_preview_test_writer | E2E Test Suite Track (Tiers 1-4) | in-progress | 7841f911-d62b-4a56-a39a-bacdc280a3c0 |
| explorer_m1_1 | teamwork_preview_explorer | M1: Status Bar Space Explorer | killed | 7838dddd-3e87-480c-999c-2ef81b612db5 |
| explorer_m1_2 | teamwork_preview_explorer | M1: Window Event Wiring Explorer | completed | 8f5c18d2-209d-49c4-9b07-7e1f12b18676 |
| explorer_m1_3 | teamwork_preview_explorer | M1: Warning & Regression Explorer | completed | f00ec3b0-5e10-4598-90cb-3c20f512ed7f |
| reviewer_m1_1 | teamwork_preview_reviewer | M1: Status Bar Reviewer 1 | completed | e60616bc-f28d-4c22-99b8-de97bbc4c76a |
| reviewer_m1_2 | teamwork_preview_reviewer | M1: Status Bar Reviewer 2 | completed | d9d18187-672f-49b9-9f7e-8921930ec2b4 |
| challenger_m1_1 | teamwork_preview_challenger | M1: Status Bar Challenger 1 | completed | c81b351b-0821-419c-8678-a48179d409af |
| challenger_m1_2 | teamwork_preview_challenger | M1: Status Bar Challenger 2 | completed | 41251701-95cc-4e22-8e86-f1250d2e0ede |
| auditor_m1_1 | teamwork_preview_auditor | M1: Forensic Auditor M1 | completed | f905eb39-8653-4206-8dba-9a0e79af277c |
| explorer_m2_1 | teamwork_preview_explorer | M2: Worker Pool Protocol Explorer | completed | 616ad38f-9f55-4eb2-8efb-898ae9b76ab9 |
| explorer_m2_2 | teamwork_preview_explorer | M2: Header Sniffing Explorer | completed | 10a9379d-eb7a-4255-8768-bf295dab476f |
| explorer_m2_3 | teamwork_preview_explorer | M2: Sandbox Guardrails Explorer | completed | 94c8067b-40dc-40db-97b0-90b212ce101c |
| worker_m2_1 | teamwork_preview_worker | M2: Worker Pool & Guardrails | completed | 08bace9e-35fc-44fa-b02c-6672d7688458 |
| reviewer_m2_1 | teamwork_preview_reviewer | M2: Reviewer 1 | in-progress | 22160c76-d3c1-4bad-be78-d9d6dd7c541c |
| reviewer_m2_2 | teamwork_preview_reviewer | M2: Reviewer 2 | in-progress | 07f57113-2557-42ed-9343-454284450906 |
| challenger_m2_1 | teamwork_preview_challenger | M2: Challenger 1 | in-progress | 14272b49-8a44-463f-b919-6f6b6d501447 |
| challenger_m2_2 | teamwork_preview_challenger | M2: Challenger 2 | in-progress | 6aaccc44-ba45-44dd-90d0-49855c911d1f |
| explorer_m3_1 | teamwork_preview_explorer | M3: GeoTIFF Pyramid Overview Explorer | completed | 08f0438a-fdcf-4ce0-8883-233198445986 |
| explorer_m3_2 | teamwork_preview_explorer | M3: DEM & RGB Contrast Explorer | completed | 8ed4d7ba-2de0-404a-927a-cfa5c484fdf3 |
| explorer_m3_3 | teamwork_preview_explorer | M3: GIS Metadata & UI Explorer | completed | 18facaa5-6d8e-4d54-9dc4-2acb7e7c4e5c |
| worker_m3_1 | teamwork_preview_worker | M3: GeoTIFF Pipeline Worker | completed | 2be03544-6822-4269-9067-e7fb79ac5b18 |
| reviewer_m3_1 | teamwork_preview_reviewer | M3: Reviewer 1 (Pyramid & Contrast) | in-progress | 1d2c62f1-a24b-4f3a-86d0-b4629b335fc2 |
| reviewer_m3_2 | teamwork_preview_reviewer | M3: Reviewer 2 (Metadata & UI) | in-progress | c8bbd5d7-8cae-4f54-8ab5-c00e64918478 |
| challenger_m3_1 | teamwork_preview_challenger | M3: Challenger 1 (Pyramid & DEM) | in-progress | c2485c25-96e8-4882-aad3-684327594fe7 |
| challenger_m3_2 | teamwork_preview_challenger | M3: Challenger 2 (Metadata & Cairo) | in-progress | 5e0a0152-7475-4c39-8284-8ae7fd548e99 |
| auditor_m3_1 | teamwork_preview_auditor | M3: Forensic Auditor | in-progress | 1e215b70-3223-4e6d-ad72-b19fc393079c |

## Succession Status
- Succession required: no (orchestrator archetype self-contained; max limit 128)
- Spawn count: 32 / 128
- Pending subagents: 1d2c62f1-a24b-4f3a-86d0-b4629b335fc2, c8bbd5d7-8cae-4f54-8ab5-c00e64918478, c2485c25-96e8-4882-aad3-684327594fe7, 5e0a0152-7475-4c39-8284-8ae7fd548e99, 1e215b70-3223-4e6d-ad72-b19fc393079c
- Predecessor: none
- Successor: none

## Active Timers
- Heartbeat cron: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae/task-246
- Safety timer: none
- On context truncation: run `manage_task(Action="list")` — re-create if missing

## Artifact Index
- /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md — Authoritative user request
- /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/DISPATCH.md — Dispatch log
- /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/BRIEFING.md — Persistent memory
- /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/progress.md — Liveness & progress tracking
- /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/plan.md — Concrete execution plan
- /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/context.md — Project context
