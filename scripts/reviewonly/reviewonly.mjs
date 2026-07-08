#!/usr/bin/env node
// PrometheOS ReviewOnly v0 — deterministic, read-only PR reviewer.
//
// No external models. No file edits. No commits. No merges.
// Reads PR metadata + diff and posts one structured ReviewOnly report comment.
//
// Runs inside GitHub Actions with `gh` available and `pull-requests: write`
// permission for posting the comment only.
//
// Safety: all `gh` calls use execFileSync with an argv array (no shell
// interpolation), and report bodies are passed through stdin via `--input -`.

import { execFileSync } from "node:child_process";

const PR = process.env.PR_NUMBER;
const REPO = process.env.REPO; // owner/repo
const GH_TOKEN = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;

// argv-array form: never goes through a shell, so diff/filename/report content
// cannot be interpreted as shell syntax.
function gh(argv, input) {
  const opts = { encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] };
  if (input !== undefined) opts.input = input;
  return execFileSync("gh", argv, opts).toString();
}

function runGhJson(argv) {
  return JSON.parse(gh(argv));
}

function getPrData() {
  return runGhJson([
    "pr",
    "view",
    String(PR),
    "--json",
    "number,title,body,additions,deletions,changedFiles,files,baseRefName,headRefName,state,url",
  ]);
}

function getDiff() {
  try {
    return gh(["pr", "diff", String(PR)]);
  } catch {
    return "";
  }
}

function getCiStatus() {
  try {
    const arr = JSON.parse(gh(["pr", "checks", String(PR), "--json", "name,state,conclusion"]));
    if (!Array.isArray(arr) || arr.length === 0) return "unavailable";
    return arr
      .map((c) => `${c.name}: ${c.state}${c.conclusion ? " (" + c.conclusion + ")" : ""}`)
      .join("; ");
  } catch {
    return "unavailable";
  }
}

// ---- analysis helpers ------------------------------------------------------

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

const EXPERIMENTAL_SURFACE_RE = /(frontend|api server|api_server|autonomous|brain|mnemosyne|plugin marketplace|cloud\/team|control plane)/i;
const AFFIRM_PROMO_RE = /(is (now )?stable alpha|stable alpha|promoted (to |into )?(stable|alpha)|alpha[- ]?promised|now part of stable alpha|becomes stable|is now stable|part of (the )?stable alpha|promote .{0,40} to stable)/i;
const NEGATION_RE = /\b(no|not|never|without|does not|doesn't|avoid|avoided|unpromoted|not part of stable alpha|future|not alpha|experimental)\b/i;
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

// Promotion/overclaim: require AFFIRMATIVE promotion language. Negated
// safety-boundary phrasing ("No frontend promotion", "experimental", "future /
// not alpha") is exempt. Uncertain matches downgrade to Warning.
function classifyPromotion(text) {
  let blocker = false;
  let warning = false;
  for (const line of text.split("\n")) {
    if (!EXPERIMENTAL_SURFACE_RE.test(line)) continue;
    if (NEGATION_RE.test(line)) continue; // safety-boundary language, ignore
    if (AFFIRM_PROMO_RE.test(line)) {
      blocker = true;
    } else if (/(stable alpha|promot|alpha)/i.test(line)) {
      warning = true;
    }
  }
  if (blocker) return "blocker";
  if (warning) return "warning";
  return null;
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

  // Touched-area classification.
  const srcTouched =
    files.some((f) => /^src\//i.test(f.path)) ||
    files.some((f) => /^cargo\.(toml|lock)$/i.test(f.path));
  const frontendTouched = files.some((f) => /^frontend\//i.test(f.path));
  const workflowTouched = files.some((f) => /^\.github\//i.test(f.path));
  const docsTouched = files.some((f) => /^docs\//i.test(f.path) || /\.md$/i.test(f.path));
  const scriptTouched = files.some((f) => /^scripts\//i.test(f.path) || /\.mjs$/i.test(f.path));
  const experimentalTouched = files.filter((f) => EXPERIMENTAL_PATH_PATTERNS.some((re) => re.test(f.path)));

  // --- blockers ------------------------------------------------------------

  const depFiles = files.filter((f) => DEPENDENCY_FILES.includes(basename(f.path)));
  if (depFiles.length > 0) {
    add(
      "Blockers",
      `Dependency files changed without explicit approval: ${depFiles.map((f) => f.path).join(", ")}. ` +
        `Per AGENTS.md, dependency changes require explicit approval.`,
    );
  }

  if (workflowTouched) {
    const removed = removedLines(diff).join("\n").toLowerCase();
    const weakened =
      /cargo test/.test(removed) ||
      /cargo clippy/.test(removed) ||
      /npm run build/.test(removed) ||
      /npm run lint/.test(removed);
    if (weakened) {
      add(
        "Blockers",
        `CI/workflow files changed and required checks appear removed (${files
          .filter((f) => /^\.github\//i.test(f.path))
          .map((f) => f.path)
          .join(", ")}). Possible CI weakening (SAFETY_GATES hard blocker).`,
      );
    } else {
      add(
        "Warnings",
        `CI/workflow files changed (${files
          .filter((f) => /^\.github\//i.test(f.path))
          .map((f) => f.path)
          .join(", ")}). Verify CI was not weakened (no check removed).`,
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

  const promotionScope = body + "\n" + addedLines(diff).join("\n");
  const promo = classifyPromotion(promotionScope);
  if (promo === "blocker") {
    add(
      "Blockers",
      `Possible promotion/overclaim of an experimental surface to stable alpha. PR text or diff contains affirmative promotion language for an experimental surface (frontend/API/autonomous/Brain/Mnemosyne). Per AGENTS.md rules, no frontend/API/autonomous promotion.`,
    );
  } else if (promo === "warning") {
    add(
      "Warnings",
      `Mentions an experimental surface together with promotion/stable-alpha language. Confirm this is not a promotion claim (negated safety-boundary language is exempt).`,
    );
  }

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

  if (experimentalTouched.length > 0) {
    add(
      "Warnings",
      `Runtime/API/frontend/harness paths touched: ${experimentalTouched
        .map((f) => f.path)
        .join(", ")}. Keep experimental surfaces out of alpha scope.`,
    );
  }

  const isDocsOnlyClaim = /^docs:/i.test(pr.title || "") || /docs-only/i.test(body);
  if (isDocsOnlyClaim && (srcTouched || frontendTouched || workflowTouched || scriptTouched)) {
    add(
      "Warnings",
      `PR claims docs-only but touches non-docs files (src/frontend/workflow/scripts). Verification must cover the changed area.`,
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
  if (/node --check/i.test(body)) claimedChecks.push("node --check");
  if (/reviewonly action|self-trigger|self trigger/i.test(body)) claimedChecks.push("ReviewOnly action self-trigger");
  if (/ci (green|pass)|all ci workflows are green|ci workflows are green/i.test(body)) claimedChecks.push("CI green");

  const missingChecks = [];
  if (srcTouched && !claimedChecks.some((c) => /cargo/i.test(c))) {
    missingChecks.push("Rust baseline (cargo fmt/check/test/clippy) not referenced for touched src/Cargo code");
  }
  if (frontendTouched && !claimedChecks.some((c) => /npm/i.test(c))) {
    missingChecks.push("Frontend build/lint not referenced for touched frontend code");
  }
  if ((workflowTouched || scriptTouched) && !claimedChecks.some((c) => /node --check|self-trigger|ci/i.test(c))) {
    missingChecks.push(
      "For workflow/script changes, expected evidence is `node --check`, ReviewOnly action self-trigger, and CI green",
    );
  }
  if (missingChecks.length > 0) {
    add("Warnings", `Missing verification evidence: ${missingChecks.join("; ")}.`);
  }

  // --- suggestions / questions ---------------------------------------------

  if (!srcTouched && !frontendTouched && fileCount <= 5 && netLines <= 200) {
    add("Suggestions", "Scope is small and within budget. Good.");
  }
  if (!body || body.trim().length < 40) {
    add("Questions", "PR body is thin. Include Summary, Safety boundary, and Verification sections per PR_TEMPLATE.md.");
  }

  return {
    findings,
    fileCount,
    netLines,
    claimedChecks,
    missingChecks,
    experimentalTouched,
    srcTouched,
    frontendTouched,
    workflowTouched,
    docsTouched,
    scriptTouched,
  };
}

// ---- report ----------------------------------------------------------------

function buildReport(pr, a, ciStatus) {
  const { findings, fileCount, netLines, claimedChecks, missingChecks, experimentalTouched } = a;
  const budget =
    fileCount <= 5 && netLines <= 200
      ? "within default budget (<=5 files, <=200 net lines)"
      : "OVER BUDGET (>5 files or >200 net lines)";

  const lines = [];
  lines.push("## PrometheOS ReviewOnly Report");
  lines.push("");
  lines.push("Mode: ReviewOnly");
  lines.push("");
  lines.push("Scope:");
  lines.push(`- Files reviewed: ${fileCount}`);
  lines.push(`- Lines changed: +${pr.additions || 0} / -${pr.deletions || 0} (net ${netLines})`);
  lines.push(`- Budget status: ${budget}`);
  lines.push(
    `- Risky path categories touched: ${experimentalTouched.length > 0 ? experimentalTouched.map((f) => f.path).join(", ") : "none"}`,
  );
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
  lines.push(
    `- Stable alpha: ${
      findings.Blockers.some((b) => /stable alpha/i.test(b))
        ? "BLOCKER — possible stable alpha scope change"
        : a.srcTouched
          ? "src/Cargo touched — verify no stable-alpha behavior change"
          : "no direct change detected"
    }`,
  );
  lines.push(
    `- Experimental surfaces: ${experimentalTouched.length > 0 ? "touched (must stay experimental)" : "none touched"}`,
  );
  lines.push(
    `- Overclaim risk: ${
      findings.Blockers.some((b) => /overclaim|promotion/i.test(b))
        ? "HIGH — possible promotion/overclaim"
        : findings.Warnings.some((w) => /promotion|stable.alpha/i.test(w))
          ? "MEDIUM — review promotion language"
          : "low"
    }`,
  );
  lines.push("");
  lines.push("Recommendation:");
  lines.push("- Wait for CI to be green before merge.");
  lines.push("- Human review required (ReviewOnly does not approve or merge).");
  if (findings.Blockers.length > 0) {
    lines.push("- Resolve blocker-level findings before merge.");
  }
  lines.push("");
  lines.push("---");
  lines.push(
    "_Deterministic ReviewOnly v0. No model invoked. Comments only; no commits, no branch writes, no merge._",
  );
  return lines.join("\n");
}

// ---- post (argv-safe, body via stdin) -------------------------------------

function findExistingComment() {
  try {
    const comments = JSON.parse(gh(["api", `repos/${REPO}/issues/${PR}/comments?per_page=100`]));
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
  const payload = JSON.stringify({ body: report });
  const existing = findExistingComment();
  if (existing) {
    gh(["api", "-X", "PATCH", `repos/${REPO}/issues/comments/${existing}`, "--input", "-"], payload);
    return "updated";
  }
  gh(["api", `repos/${REPO}/issues/${PR}/comments`, "--input", "-"], payload);
  return "created";
}

function postError(msg) {
  try {
    const payload = JSON.stringify({
      body: `## PrometheOS ReviewOnly Report\n\nMode: ReviewOnly\n\nFindings:\n- Warnings: ${msg}\n\nRecommendation:\n- Human review required.`,
    });
    gh(["api", `repos/${REPO}/issues/${PR}/comments`, "--input", "-"], payload);
  } catch {
    /* never block CI */
  }
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
  const a = analyze(pr, diff);
  const report = buildReport(pr, a, ciStatus);
  const action = postReport(report);
  console.log(`ReviewOnly report ${action} for PR #${PR}.`);
  process.exit(0);
} catch (err) {
  postError(`ReviewOnly v0 encountered an internal error: ${err && err.message ? err.message : String(err)}`);
  process.exit(0);
}
