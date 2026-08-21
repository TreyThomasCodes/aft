/// <reference path="../bun-test.d.ts" />

import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";

import {
  DOCTOR_BUILD_BREAKER_RESET_COMMAND,
  formatBuildBreakerSuspension,
  readBuildBreakerSuspensions,
  resetBuildBreakerSuspension,
} from "./build-breaker.js";

function fixture(nowMs = 1_000_000): { storage: string; root: string; fingerprint: string } {
  const storage = mkdtempSync(join(tmpdir(), "aft-cli-breaker-"));
  const directory = join(storage, "callgraph", "root-key");
  mkdirSync(directory, { recursive: true });
  const database = new DatabaseSync(join(directory, "build-breaker.sqlite"));
  database.exec(`
    CREATE TABLE breaker_records (
      root_id TEXT NOT NULL,
      domain TEXT NOT NULL,
      corpus_fingerprint TEXT NOT NULL,
      configuration_version TEXT NOT NULL,
      zero_credit_deaths INTEGER NOT NULL DEFAULT 0,
      credited_deaths INTEGER NOT NULL DEFAULT 0,
      in_build_burn_ms INTEGER NOT NULL DEFAULT 0,
      suspended_reason TEXT,
      suspended_since_ms INTEGER,
      suspended_until_ms INTEGER,
      PRIMARY KEY(root_id, domain, corpus_fingerprint)
    );
  `);
  const root = "/project/root";
  const fingerprint = "corpus-a";
  database
    .prepare(
      `INSERT INTO breaker_records(
        root_id, domain, corpus_fingerprint, configuration_version,
        zero_credit_deaths, credited_deaths, suspended_reason,
        suspended_since_ms, suspended_until_ms
      ) VALUES (?, ?, ?, 'v1', 2, 1, 'zero_credit_death_limit', ?, ?)`,
    )
    .run(root, "callgraph_cold", fingerprint, nowMs - 5_000, nowMs + 5_000);
  database.close();
  return { storage, root, fingerprint };
}

describe("durable build-breaker doctor surfaces", () => {
  test("renders the persisted domain, counter, age, reason, and reset command", () => {
    const nowMs = 1_000_000;
    const { storage, root, fingerprint } = fixture(nowMs);

    const [suspension] = readBuildBreakerSuspensions(storage, nowMs);

    expect(suspension).toEqual({
      root,
      domain: "callgraph_cold",
      reason: "zero_credit_death_limit",
      deathCount: 3,
      ageS: 5,
      fingerprint,
    });
    expect(suspension).toBeDefined();
    expect(formatBuildBreakerSuspension(suspension!)).toBe(
      `  build suspended: root=/project/root domain=callgraph_cold deaths=3 age_s=5 reason=zero_credit_death_limit; reset with \`${DOCTOR_BUILD_BREAKER_RESET_COMMAND} --root /project/root --domain callgraph_cold --fingerprint corpus-a\``,
    );
  });

  test("TTL-lift hides the trip while retaining counters until an explicit reset", () => {
    const nowMs = 1_000_000;
    const { storage, root, fingerprint } = fixture(nowMs);

    expect(readBuildBreakerSuspensions(storage, nowMs + 5_001)).toEqual([]);
    const databasePath = join(storage, "callgraph", "root-key", "build-breaker.sqlite");
    let database = new DatabaseSync(databasePath, { readOnly: true });
    let row = database
      .prepare("SELECT zero_credit_deaths, credited_deaths FROM breaker_records WHERE root_id = ?")
      .get(root) as { zero_credit_deaths: number; credited_deaths: number };
    database.close();
    expect(row).toEqual({ zero_credit_deaths: 2, credited_deaths: 1 });

    expect(
      resetBuildBreakerSuspension(storage, {
        root,
        domain: "callgraph_cold",
        fingerprint,
      }),
    ).toBe(1);

    database = new DatabaseSync(databasePath, { readOnly: true });
    row = database
      .prepare("SELECT zero_credit_deaths, credited_deaths FROM breaker_records WHERE root_id = ?")
      .get(root) as { zero_credit_deaths: number; credited_deaths: number };
    database.close();
    expect(row).toEqual({ zero_credit_deaths: 0, credited_deaths: 0 });
  });
});
