// SPDX-License-Identifier: GPL-3.0-or-later

//! Hermes Phase 1 Comprehensive Opaque-Box E2E Test Suite.
//!
//! Tiers:
//! - Tier 1: Happy Path Isolation Tests (>=5 tests per feature for F1..F14, 70 tests)
//! - Tier 2: Boundary Value Analysis & Adversarial Tests (>=5 tests per feature for F1..F14, 70 tests)
//! - Tier 3: Pairwise Interacting Combinations (>=20 interaction tests)
//! - Tier 4: Realistic Real-World Workload Scenarios (>=5 application scenarios)

mod common;
mod fixtures;

#[path = "e2e/tier1_isolated.rs"]
mod tier1_isolated;

#[path = "e2e/tier2_boundaries.rs"]
mod tier2_boundaries;

#[path = "e2e/tier3_pairwise.rs"]
mod tier3_pairwise;

#[path = "e2e/tier4_scenarios.rs"]
mod tier4_scenarios;
