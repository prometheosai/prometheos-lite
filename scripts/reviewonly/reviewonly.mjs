#!/usr/bin/env node
// PrometheOS ReviewOnly v0 — deterministic, read-only PR reviewer.
//
// No external models. No file edits. No commits. No merges.
// Reads PR metadata + diff and the repo's agent/safety docs, then posts one
// structured ReviewOnly report comment.
//
// Designed to run inside GitHub Actions with `gh` available and
// `pull-requests: write` permission for posting the comment only.

import { execSync } from "node:child_process";

const PR = process.env.PR_NUMBER;
const GH_TOKEN = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;

function gh(args, input) {
  const opts = { encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] };
  if (input !== undefined) opts.input = input;
  return execSync(`gh ${args}`, opts).toString();
}

// ---- helpers ---------------------------------------------------------------

function runGhJson(args) {
  return JSON.parse(gh(args));
}

function getPrData() {
  return runGhJson(
    `pr view ${PR} --json number,title,body,additions,deletions,changedFiles,files,baseRefName,headRefName,state,url`,
  );
}

function getDiff() {
  try {
    return gh(`pr diff ${PR}`);
  } catch {
    return "";
  }
}

function getCiStatus() {
  try {
    const out = gh(`pr checks ${PR} --json name,state,conclusion 2>/dev/null`);
    const arr = JSON.parse(out);
    if (!Array.isArray(arr) || arr.length === 0) return "unavailable";
    return arr
      .map((c) => `${c.name}: ${c.state}${c.conclusion ? " (" + c.conclusion + ")" : ""}`)
      .join("; ");
  } catch {
    return "unavailable";
  }
}

// ---- analysis --------------------------------------------------------------

const DEPENDENCY_FILES = [
  "cargo.toml",
  "cargo.lock",
  "package.json",
  "package-lock.json",
  "yarn.lock",
  "pnpm-lock.yaml",
  "gemfile",
  "gemfile.lock",
  "go.mod",
  "go.sum",
  "requirements.txt",
  "pyproject.toml",
  "pom.xml",
  "build.gradle",
];

const EXPERIMENTAL_PATH_PATTERNS = [
  /^src\/api\//i,
  /^src\/serve\//i,
  /^src\/flow\//i,
  /^src\/harness\//i,
  /^src\/brain\//i,
  /^src\/mnemosyne\//i,
  /^frontend\//i,
  /^benchmark/i,
  /^src\/llm\//i,
];

const SECRET_RE = [
  /-----BEGIN (?:RSA |EC |OPENSSH |PGP |DSA )?PRIVATE KEY-----/,
  /sk-[A-Za-z0-9]{20,}/,
  /AKIA[0-9A-Z]{16}/,
  /(api[_-]?key|secret|token|password|passwd|private[_-]?key)\s*[=:]\s*["'][^"']{8,}["']/i,
  /ghp_[A-Za-z0-9]{30,}/,
  /github_pat_[A-Za-z0-9_]{20,}/,
];

const PROMOTION_RE = /(promote|now stable|is now stable|stable alpha|promoted to stable|alpha-promised|alpha promised)/i;
const EXPERIMENTAL_SURFACE_RE = /(frontend|api server|api_server|autonomous|brain|mnemosyne|plugin marketplace|cloud\/team|control plane)/i;
const BENCHMARK_RE = /(benchmark|outperform|sota|state-of-the-art|beats|surpass|better than \w+ by)/i;
const EVIDENCE_RE = /(verified|evidence|cargo test|cargo clippy|npm run build|npm ci|validation|ran the checks)/i;

function addedLines(diff) {
  return diff
    .split("\n")
    .filter((l) => l.startsWith("+") && !l.startsWith("+++"))
    .map((l) => l.slice(1));
}

function removedLines(diff) {
  return diff
    .split("\n")
    .filter((l) => l.startsWith("-") && !l.startsWith("---"))
    .map((l) => l.slice(1));
}

function basename(p) {
  return p.split("/").pop().toLowerCase();
}

function analyze(pr, diff) {
  const findings = { Blockers: [], Warnings: [], Suggestions: [], Questions: [] };
  const add = (sev, text) => findings[sev].push(text);

  const files = (pr.files || []).map((f) => ({
    path: f.path,
    status: f.status,
    additions: f.additions || 0,
    deletions: f.deletions || 0,
  }));
  const fileCount = pr.changedFiles || files.length;
  const netLines = (pr.additions || 0) - (pr.deletions || 0);
  const body = pr.body || "";

  // --- blockers ------------------------------------------------------------

  const depFiles = files.filter((f) =>
    DEPENDENCY_FILES.includes(basename(f.path)) ||
    /\.(csproj|toml|lock|gradle)$/i.test(f.path) && DEPENDENCY_FILES.some((d) => f.path.toLowerCase().endsWith(d)),
  );
  if (depFiles.length > 0) {
    add(
      "Blockers",
      `Dependency files changed without explicit approval: ${depFiles.map((f) => f.path).join(", ")}. ` +
        `Per AGENTS.md, dependency changes require explicit approval.`,
    );
  }

  const workflowFiles = files.filter((f) => /^\.github\//i.test(f.path));
  if (workflowFiles.length > 0) {
    const removed = removedLines(diff).join("\n").toLowerCase();
    const weakened =
      /cargo test/.test(removed) ||
      /cargo clippy/.test(removed) ||
      /npm run build/.test(removed) ||
      /npm run lint/.test(removed);
    if (weakened) {
      add(
        "Blockers",
        `CI/workflow files changed and required checks appear removed: ${workflowFiles
          .map((f) => f.path)
          .join(", ")}. Possible CI weakening (SAFETY_GATES hard blocker).`,
      );
    } else {
      add(
        "Warnings",
        `CI/workflow files changed: ${workflowFiles
          .map((f) => f.path)
          .join(", ")}. Verify CI was not weakened (no check removed).`,
      );
    }
  }

  const secretHits = [];
  for (const line of addedLines(diff)) {
    for (const re of SECRET_RE) {
      if (re.test(line)) secretHits.push(line.trim().slice(0, 80));
    }
  }
  if (secretHits.length > 0) {
    add(
      "Blockers",
      `Possible secret/credential in diff: ${secretHits
        .slice(0, 3)
        .map((s) => `"${s}"`)
        .join(", ")}. Do not commit secrets (SAFETY_GATES hard blocker).`,
    );
  }

  const conflictMarkers = addedLines(diff).filter((l) =>
    /^(<<<<<<<|=======|>>>>>>>|\|\|\|\|\|\|\|)/.test(l),
  );
  if (conflictMarkers.length > 0) {
    add("Blockers", `Source-of-truth conflict markers detected in diff (${conflictMarkers.length} line(s)).`);
  }

  // promotion / overclaim (blocker if explicit promotion phrasing)
  const promotionScope = body + "\n" + addedLines(diff).join("\n");
  if (PROMOTION_RE.test(promotionScope) && EXPERIMENTAL_SURFACE_RE.test(promotionScope)) {
    add(
      "Blockers",
      `Possible promotion/overclaim of an experimental surface to stable alpha. PR text or diff mentions stable alpha together with an experimental surface (frontend/API/autonomous/Brain/Mnemosyne). Per AGENTS.md rules, no frontend/API/autonomous promotion.`,
    );
  }

  // benchmark claims without evidence
  if (BENCHMARK_RE.test(promotionScope) && !EVIDENCE_RE.test(promotionScope)) {
    add(
      "Blockers",
      `Benchmark/performance claim detected without verification evidence markers. Per AGENTS.md, benchmark claims require completed validation evidence.`,
    );
  }

  // --- warnings -------------------------------------------------------------

  if (fileCount > 5) {
    add("Warnings", `Changed files (${fileCount}) exceed the default 5-file budget. Prefer small PRs.`);
  }
  if (netLines > 200) {
    add("Warnings", `Net lines changed (${netLines}) exceed the default 200-line budget. Prefer small PRs.`);
  }

  const experimentalTouched = files.filter((f) =>
    EXPERIMENTAL_PATH_PATTERNS.some((re) => re.test(f.path)),
  );
  if (experimentalTouched.length > 0) {
    add(
      "Warnings",
      `Runtime/API/frontend/harness paths touched: ${experimentalTouched
        .map((f) => f.path)
        .join(", ")}. Keep experimental surfaces out of alpha scope.`,
    );
  }

  const isDocsOnlyClaim =
    /^docs:/i.test(pr.title || "") || /docs-only/i.test(body);
  const runtimeTouched = files.some(
    (f) => /^src\//i.test(f.path) || /^frontend\//i.test(f.path) || /^\.github\//i.test(f.path),
  );
  if (isDocsOnlyClaim && runtimeTouched) {
    add(
      "Warnings",
      `PR claims docs-only but touches runtime/API/frontend/workflow files. Verification must cover the changed area.`,
    );
  }

  // --- verification evidence ------------------------------------------------

  const claimedChecks = [];
  if (/cargo test/i.test(body)) claimedChecks.push("cargo test");
  if (/cargo clippy/i.test(body)) claimedChecks.push("cargo clippy");
  if (/cargo (fmt|check)/i.test(body)) claimedChecks.push("cargo fmt/check");
  if (/npm (run )?build/i.test(body)) claimedChecks.push("npm run build");
  if (/npm (run )?lint/i.test(body)) claimedChecks.push("npm run lint");
  if (/npm ci/i.test(body)) claimedChecks.push("npm ci");

  const missingChecks = [];
  if (runtimeTouched && !claimedChecks.some((c) => /cargo/i.test(c))) {
    missingChecks.push("Rust baseline (cargo fmt/check/test/clippy) not referenced for touched src code");
  }
  if (experimentalTouched.some((f) => /^frontend\//i.test(f.path)) && !claimedChecks.some((c) => /npm/i.test(c))) {
    missingChecks.push("Frontend build/lint not referenced for touched frontend code");
  }
  if (missingChecks.length > 0) {
    add("Warnings", `Missing verification evidence: ${missingChecks.join("; ")}.`);
  }

  // --- suggestions / questions ---------------------------------------------

  if (experimentalTouched.length === 0 && runtimeTouched === false && fileCount <= 5 && netLines <= 200) {
    add("Suggestions", "Scope is small and within budget. Good.");
  }
  if (!body || body.trim().length < 40) {
    add("Questions", "PR body is thin. Include Summary, Safety boundary, and Verification sections per PR_TEMPLATE.md.");
  }

  return { findings, fileCount, netLines, claimedChecks, missingChecks, experimentalTouched };
}

// ---- report ----------------------------------------------------------------

function buildReport(pr, analysis, ciStatus) {
  const { findings, fileCount, netLines, claimedChecks, missingChecks, experimentalTouched } = analysis;
  const budget = fileCount <= 5 && netLines <= 200 ? "within default budget (<=5 files, <=200 net lines)" : "OVER BUDGET (>5 files or >200 net lines)";

  const lines = [];
  lines.push("## PrometheOS ReviewOnly Report");
  lines.push("");
  lines.push("Mode: ReviewOnly");
  lines.push("");
  lines.push("Scope:");
  lines.push(`- Files reviewed: ${fileCount}`);
  lines.push(`- Lines changed: +${pr.additions || 0} / -${pr.deletions || 0} (net ${netLines})`);
  lines.push(`- Budget status: ${budget}`);
  lines.push(`- Risky path categories touched: ${experimentalTouched.length > 0 ? experimentalTouched.map((f) => f.path).join(", ") : "none"}`);
  lines.push("");
  lines.push("Findings:");
  for (const sev of ["Blockers", "Warnings", "Suggestions", "Questions"]) {
    const items = findings[sev];
    if (items.length === 0) {
      lines.push(`- ${sev}: none`);
    } else {
      for (const it of items) lines.push(`- ${sev}: ${it}`);
    }
  }
  lines.push("");
  lines.push("Verification evidence:");
  lines.push(`- Claimed checks: ${claimedChecks.length > 0 ? claimedChecks.join(", ") : "none stated"}`);
  lines.push(`- Missing checks: ${missingChecks.length > 0 ? missingChecks.join("; ") : "none detected"}`);
  lines.push(`- CI status: ${ciStatus}`);
  lines.push("");
  lines.push("Product boundary check:");
  const stableAlphaImpact =
    analysis.experimentalTouched.length === 0 && !/^src\//i.test("") ? "none detected" : "see experimental surface findings";
  lines.push(`- Stable alpha: ${findings.Blockers.some((b) => /stable alpha/i.test(b)) ? "BLOCKER — possible stable alpha scope change" : "no direct change detected"}`);
  lines.push(`- Experimental surfaces: ${experimentalTouched.length > 0 ? "touched (must stay experimental)" : "none touched"}`);
  lines.push(`- Overclaim risk: ${findings.Blockers.some((b) => /overclaim|promotion/i.test(b)) ? "HIGH — possible promotion/overclaim" : "low"}`);
  lines.push("");
  lines.push("Recommendation:");
  lines.push("- Wait for CI to be green before merge.");
  lines.push("- Human review required (ReviewOnly does not approve or merge).");
  if (findings.Blockers.length > 0) {
    lines.push("- Resolve blocker-level findings before merge.");
  }
  lines.push("");
  lines.push("---");
  lines.push("_Deterministic ReviewOnly v0. No model invoked. Comments only; no commits, no branch writes, no merge._");
  return lines.join("\n");
}

// ---- post (with dedupe) ----------------------------------------------------

function findExistingComment() {
  try {
    const comments = JSON.parse(gh(`api repos/${process.env.REPO}/issues/${PR}/comments?per_page=100`));
    const marker = "## PrometheOS ReviewOnly Report";
    for (const c of comments) {
      if ((c.user?.login || "").includes("bot") && (c.body || "").includes(marker)) {
        return c.id;
      }
    }
  } catch {
    /* ignore */
  }
  return null;
}

function postReport(report) {
  const existing = findExistingComment();
  if (existing) {
    gh(`api -X PATCH repos/${process.env.REPO}/issues/comments/${existing} -f body=${JSON.stringify(report)}`);
    return "updated";
  }
  gh(`pr comment ${PR} --body ${JSON.stringify(report)}`);
  return "created";
}

// ---- main ------------------------------------------------------------------

try {
  if (!PR) {
    console.error("PR_NUMBER not set");
    process.exit(0);
  }
  const pr = getPrData();
  const diff = getDiff();
  const ciStatus = getCiStatus();
  const analysis = analyze(pr, diff);
  const report = buildReport(pr, analysis, ciStatus);
  const action = postReport(report);
  console.log(`ReviewOnly report ${action} for PR #${PR}.`);
  process.exit(0);
} catch (err) {
  const msg = `ReviewOnly v0 encountered an internal error: ${err && err.message ? err.message : String(err)}`;
  try {
    gh(`pr comment ${PR} --body ${JSON.stringify("## PrometheOS ReviewOnly Report\n\nMode: ReviewOnly\n\nFindings:\n- Warnings: " + msg + "\n\nRecommendation:\n- Human review required.")}`);
  } catch {
    /* give up silently; never block CI */
  }
  process.exit(0);
}
