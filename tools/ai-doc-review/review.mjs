#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const repoRoot = process.cwd();
const reportPath = process.env.DOC_REVIEW_REPORT_PATH || "doc-review-report.json";
const summaryPath = process.env.DOC_REVIEW_SUMMARY_PATH || "doc-review-summary.md";
const failOnError = (process.env.DOC_REVIEW_FAIL_ON_ERROR || "true").toLowerCase() === "true";
const failOnAiError = (process.env.DOC_REVIEW_FAIL_ON_AI_ERROR || "false").toLowerCase() === "true";
const failConfidence = Number(process.env.DOC_REVIEW_FAIL_CONFIDENCE || "0.85");
const maxBookChars = Number(process.env.DOC_REVIEW_MAX_BOOK_CHARS || "120000");

function readText(filePath) {
  return fs.readFileSync(path.join(repoRoot, filePath), "utf8");
}

function fileExists(filePath) {
  return fs.existsSync(path.join(repoRoot, filePath));
}

function discoverBookSrc() {
  if (!fileExists("book.toml")) return "training";
  const bookToml = readText("book.toml");
  const match = bookToml.match(/^\s*src\s*=\s*["']([^"']+)["']/m);
  return match?.[1] || "training";
}

function walkMarkdown(dir) {
  const absoluteDir = path.join(repoRoot, dir);
  if (!fs.existsSync(absoluteDir)) return [];
  const results = [];
  for (const entry of fs.readdirSync(absoluteDir, { withFileTypes: true })) {
    const absolute = path.join(absoluteDir, entry.name);
    const relative = path.relative(repoRoot, absolute).replaceAll(path.sep, "/");
    if (entry.isDirectory()) {
      results.push(...walkMarkdown(relative));
    } else if (entry.isFile() && entry.name.endsWith(".md")) {
      results.push(relative);
    }
  }
  return results.sort();
}

function discoverMarkdownFiles(bookSrc) {
  const summary = path.join(bookSrc, "SUMMARY.md").replaceAll(path.sep, "/");
  const ordered = [];
  const seen = new Set();

  function add(file) {
    const normalized = normalizeRepoPath(file);
    if (!seen.has(normalized) && fileExists(normalized)) {
      seen.add(normalized);
      ordered.push(normalized);
    }
  }

  add(summary);

  if (fileExists(summary)) {
    const summaryText = readText(summary);
    const linkPattern = /\]\(([^)]+\.md)(?:#[^)]+)?\)/g;
    for (const match of summaryText.matchAll(linkPattern)) {
      const linked = path.normalize(path.join(bookSrc, match[1]));
      add(linked);
    }
  }

  for (const file of walkMarkdown(bookSrc)) add(file);
  return ordered;
}

function normalizeRepoPath(filePath) {
  return String(filePath || "").replaceAll("\\", "/").replace(/^\.\//, "");
}

function parseChangedLinesFromPatch(patch) {
  const changedLines = new Set();
  let newLine = 0;

  for (const line of String(patch || "").split("\n")) {
    const hunk = line.match(/^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    if (hunk) {
      newLine = Number(hunk[1]);
      continue;
    }

    if (line.startsWith("+++")) continue;
    if (line.startsWith("+")) {
      changedLines.add(newLine);
      newLine += 1;
    } else if (line.startsWith("-")) {
      continue;
    } else if (line.startsWith(" ")) {
      newLine += 1;
    }
  }

  return changedLines;
}

function githubApiHeaders() {
  const token = process.env.DOC_REVIEW_GITHUB_TOKEN || process.env.GITHUB_TOKEN;
  const headers = {
    "Accept": "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28"
  };
  if (token) headers.Authorization = `Bearer ${token}`;
  return headers;
}

function githubApiUrl(pathname) {
  const apiBaseUrl = (process.env.GITHUB_API_URL || "https://api.github.com").replace(/\/$/, "");
  return `${apiBaseUrl}${pathname}`;
}

async function fetchPullRequestFiles(owner, repo, pullNumber) {
  const files = [];
  for (let page = 1; ; page += 1) {
    const response = await fetch(githubApiUrl(`/repos/${owner}/${repo}/pulls/${pullNumber}/files?per_page=100&page=${page}`), {
      headers: githubApiHeaders()
    });
    if (!response.ok) {
      const body = await response.text();
      throw new Error(`GitHub API request failed (${response.status}): ${body}`);
    }

    const pageFiles = await response.json();
    if (!Array.isArray(pageFiles)) throw new Error("GitHub API response did not contain pull request files.");
    files.push(...pageFiles);
    if (pageFiles.length < 100) break;
  }
  return files;
}

async function resolveReviewScope(files) {
  const eventName = process.env.GITHUB_EVENT_NAME;
  if (eventName !== "pull_request" && eventName !== "pull_request_target") {
    return { enabled: false, reason: "No pull request event detected." };
  }

  const eventPath = process.env.GITHUB_EVENT_PATH;
  if (!eventPath || !fs.existsSync(eventPath)) {
    throw new Error("Cannot determine PR diff: GITHUB_EVENT_PATH is not set or does not exist.");
  }

  const event = JSON.parse(fs.readFileSync(eventPath, "utf8"));
  const pullNumber = event.pull_request?.number || event.number;
  const repository = event.repository?.full_name || process.env.GITHUB_REPOSITORY;
  if (!pullNumber || !repository) throw new Error("Cannot determine PR diff: pull request number or repository is missing.");

  const [owner, repo] = repository.split("/");
  const reviewedFiles = new Set(files.map(normalizeRepoPath));
  const changedLinesByFile = new Map();
  const prFiles = await fetchPullRequestFiles(owner, repo, pullNumber);

  for (const prFile of prFiles) {
    const file = normalizeRepoPath(prFile.filename);
    if (!reviewedFiles.has(file) || !prFile.patch) continue;

    const changedLines = parseChangedLinesFromPatch(prFile.patch);
    if (changedLines.size > 0) changedLinesByFile.set(file, changedLines);
  }

  return {
    enabled: true,
    reason: `Pull request #${pullNumber}`,
    owner,
    repo,
    pullNumber,
    changedLinesByFile
  };
}

function compactLineNumbers(lines) {
  const sorted = [...lines].sort((a, b) => a - b);
  const ranges = [];
  for (let index = 0; index < sorted.length; index += 1) {
    const start = sorted[index];
    let end = start;
    while (index + 1 < sorted.length && sorted[index + 1] === end + 1) {
      index += 1;
      end = sorted[index];
    }
    ranges.push(start === end ? String(start) : `${start}-${end}`);
  }
  return ranges.join(", ");
}

function countReviewScopeLines(reviewScope) {
  if (!reviewScope.enabled) return 0;
  return [...reviewScope.changedLinesByFile.values()].reduce((total, lines) => total + lines.size, 0);
}

function formatReviewScope(reviewScope) {
  if (!reviewScope.enabled) return `No PR diff scope detected (${reviewScope.reason}); findings may cover any reviewed file.`;
  if (reviewScope.changedLinesByFile.size === 0) return "The PR does not add or modify any lines in the reviewed Markdown files. Return {\"findings\":[]}.";

  const entries = [...reviewScope.changedLinesByFile.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([file, lines]) => `- ${file}: ${compactLineNumbers(lines)}`);
  return `Only return findings whose file and line are in these PR changed lines:\n${entries.join("\n")}`;
}

function findingMatchesReviewScope(finding, reviewScope) {
  if (!reviewScope.enabled) return true;
  const file = normalizeRepoPath(finding.file);
  const line = Number(finding.line);
  return Boolean(file && Number.isInteger(line) && reviewScope.changedLinesByFile.get(file)?.has(line));
}

function filterFindingsToReviewScope(findings, reviewScope) {
  return findings.filter((finding) => findingMatchesReviewScope(finding, reviewScope));
}

function serializeReviewScope(reviewScope) {
  if (!reviewScope.enabled) return { enabled: false, reason: reviewScope.reason };
  return {
    enabled: true,
    reason: reviewScope.reason,
    repository: `${reviewScope.owner}/${reviewScope.repo}`,
    pullNumber: reviewScope.pullNumber,
    changedLines: Object.fromEntries(
      [...reviewScope.changedLinesByFile.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([file, lines]) => [file, [...lines].sort((a, b) => a - b)])
    )
  };
}

function buildBookPayload(files) {
  const chunks = [];
  let total = 0;
  for (const file of files) {
    const text = readText(file);
    const chunk = `<file path="${file}">\n${text}\n</file>`;
    if (total + chunk.length > maxBookChars) {
      chunks.push(`\n<!-- Skipped ${file}: book payload exceeded ${maxBookChars} characters. -->`);
      continue;
    }
    chunks.push(chunk);
    total += chunk.length;
  }
  return chunks.join("\n\n");
}

function normalizeFindingText(value) {
  return String(value || "")
    .replace(/^Suggestion:\s*/i, "")
    .replace(/^Change to:\s*/i, "")
    .replace(/^[-*]\s+/, "")
    .replace(/^['"]|['"]$/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function normalizeAiFinding(finding) {
  const message = String(finding.message || "");
  const suggestion = String(finding.suggestion || "");
  const quote = String(finding.quote || "");
  const reviewText = `${message} ${suggestion}`;

  if (/\b(no change needed|no further action|correct use|correctly done|correct phrasing|correct here|this is correct|correct as is)\b/i.test(reviewText)) {
    return undefined;
  }

  // If the replacement is identical to the quoted text, the finding is not actionable.
  if (suggestion && normalizeFindingText(suggestion) === normalizeFindingText(quote)) {
    return undefined;
  }

  let severity = ["suggestion", "warning", "error"].includes(finding.severity) ? finding.severity : "warning";
  if (severity === "error" && !failOnAiError) severity = "warning";

  const confidence = typeof finding.confidence === "number" ? Math.max(0, Math.min(1, finding.confidence)) : 0.75;
  const line = Number(finding.line);
  return {
    source: "ai",
    category: finding.category || "consistency",
    ruleId: finding.ruleId || undefined,
    severity,
    confidence,
    file: finding.file ? normalizeRepoPath(finding.file) : undefined,
    line: Number.isInteger(line) && line > 0 ? line : undefined,
    quote: finding.quote,
    message: finding.message,
    suggestion: finding.suggestion
  };
}

function extractJson(content) {
  const trimmed = content.trim();
  if (trimmed.startsWith("{")) return JSON.parse(trimmed);
  const fenced = trimmed.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
  if (fenced) return JSON.parse(fenced[1]);
  throw new Error("AI response was not JSON.");
}

function resolveAiConfig() {
  const configuredBaseUrl = process.env.AI_DOC_REVIEW_BASE_URL || process.env.OPENAI_BASE_URL;
  const usesGitHubModels = configuredBaseUrl?.includes("models.github.ai") || (!configuredBaseUrl && process.env.GITHUB_TOKEN && !process.env.OPENAI_API_KEY);
  const defaultBaseUrl = usesGitHubModels ? "https://models.github.ai/inference" : "https://api.openai.com/v1";
  const defaultModel = usesGitHubModels ? "openai/gpt-4.1-mini" : "gpt-4.1-mini";
  const apiKey = process.env.AI_DOC_REVIEW_API_KEY || process.env.GITHUB_TOKEN || process.env.OPENAI_API_KEY;

  if (!apiKey) return undefined;

  return {
    apiKey,
    model: process.env.AI_DOC_REVIEW_MODEL || process.env.OPENAI_MODEL || defaultModel,
    baseUrl: (configuredBaseUrl || defaultBaseUrl).replace(/\/$/, ""),
    provider: usesGitHubModels ? "GitHub Models" : "OpenAI-compatible"
  };
}

async function runAiReview(files, styleGuide, reviewScope) {
  const aiConfig = resolveAiConfig();
  if (!aiConfig) {
    return { skipped: true, reason: "AI_DOC_REVIEW_API_KEY, GITHUB_TOKEN, or OPENAI_API_KEY is not set.", findings: [] };
  }

  const bookPayload = buildBookPayload(files);
  const reviewScopeText = formatReviewScope(reviewScope);
  const { apiKey, model, baseUrl, provider } = aiConfig;

  const systemPrompt = `You are a meticulous technical editor for an mdBook about Embedded Rust on Espressif hardware.
Review the whole book as one coherent document. Check spelling, grammar, terminology consistency, formatting consistency, voice/tone consistency, and cross-chapter continuity.
When a PR diff scope is provided, only report actionable findings on PR changed lines. You may use the unchanged surrounding content for context, but do not report findings outside that scope.
Return only actionable findings where the quoted text should be changed. Do not report correct usage, and never use suggestions like "No change needed".
Omit findings when the suggested replacement is identical to the quoted text.
Do not rewrite whole sections. Do not flag code examples, URLs, commands, fenced code blocks, inline code, Markdown link destinations, or exact package/repository names unless the surrounding prose is wrong.
Do not invent terminology rules that are not present in the style guide.
Return strict JSON with this shape: {"findings":[{"category":"spelling|grammar|terminology|formatting|consistency|voice|continuity","severity":"suggestion|warning|error","confidence":0.0,"file":"path","line":1,"quote":"exact text","message":"why this matters","suggestion":"specific replacement or action"}]}.
Use severity "error" only for high-confidence factual style violations explicitly covered by the style guide, or clear spelling errors. Limit output to the 50 most useful findings.`;

  const userPrompt = `Style guide:\n${styleGuide}\n\nPR diff scope:\n${reviewScopeText}\n\nBook files in reading order:\n${bookPayload}`;

  const response = await fetch(`${baseUrl}/chat/completions`, {
    method: "POST",
    headers: {
      "Authorization": `Bearer ${apiKey}`,
      "Content-Type": "application/json"
    },
    body: JSON.stringify({
      model,
      temperature: 0.1,
      max_tokens: Number(process.env.DOC_REVIEW_MAX_TOKENS || "4000"),
      response_format: { type: "json_object" },
      messages: [
        { role: "system", content: systemPrompt },
        { role: "user", content: userPrompt }
      ]
    })
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(`${provider} API request failed (${response.status}): ${body}`);
  }

  const data = await response.json();
  const content = data.choices?.[0]?.message?.content;
  if (!content) throw new Error(`${provider} API response did not contain message content.`);

  const parsed = extractJson(content);
  const findings = Array.isArray(parsed.findings) ? parsed.findings.map(normalizeAiFinding).filter(Boolean) : [];
  return { skipped: false, provider, model, findings };
}

function commandEscape(value) {
  return String(value ?? "")
    .replaceAll("%", "%25")
    .replaceAll("\r", "%0D")
    .replaceAll("\n", "%0A");
}

function annotate(findings) {
  for (const finding of findings) {
    const level = finding.severity === "error" ? "error" : "warning";
    const properties = [];
    if (finding.file) properties.push(`file=${commandEscape(finding.file)}`);
    if (finding.line) properties.push(`line=${commandEscape(finding.line)}`);
    if (finding.category) properties.push(`title=${commandEscape(`docs:${finding.source || "review"}:${finding.category}`)}`);
    const propertyText = properties.length > 0 ? ` ${properties.join(",")}` : "";
    const message = `${finding.message || "Documentation review finding"}${finding.suggestion ? ` Suggestion: ${finding.suggestion}` : ""}`;
    console.log(`::${level}${propertyText}::${commandEscape(message)}`);
  }
}

function summarize(findings, aiResult, files, reviewScope, excludedFindingCount) {
  const counts = findings.reduce((acc, finding) => {
    acc[finding.severity] = (acc[finding.severity] || 0) + 1;
    return acc;
  }, {});

  const lines = [];
  lines.push("# Documentation review");
  lines.push("");
  lines.push(`Reviewed ${files.length} Markdown files.`);
  if (reviewScope.enabled) {
    lines.push(`Actionable scope: ${countReviewScopeLines(reviewScope)} changed line(s) in ${reviewScope.changedLinesByFile.size} Markdown file(s) from ${reviewScope.reason}.`);
  }
  if (excludedFindingCount > 0) {
    lines.push(`Ignored ${excludedFindingCount} finding(s) outside the PR diff.`);
  }
  lines.push(`Findings: ${findings.length} (${counts.error || 0} errors, ${counts.warning || 0} warnings, ${counts.suggestion || 0} suggestions).`);
  if (aiResult.skipped) {
    lines.push(`AI review skipped: ${aiResult.reason}`);
  } else {
    lines.push(`AI provider: ${aiResult.provider || "OpenAI-compatible"}`);
    lines.push(`AI model: ${aiResult.model}`);
  }
  lines.push("");

  if (findings.length === 0) {
    lines.push("No documentation findings.");
  } else {
    lines.push("| Severity | Source | File | Line | Category | Finding |");
    lines.push("| --- | --- | --- | ---: | --- | --- |");
    for (const finding of findings.slice(0, 50)) {
      const message = (finding.message || "").replaceAll("|", "\\|").replace(/\s+/g, " ");
      const suggestion = finding.suggestion ? ` Suggested: ${finding.suggestion}` : "";
      lines.push(`| ${finding.severity} | ${finding.source} | ${finding.file || ""} | ${finding.line || ""} | ${finding.category || ""} | ${message}${suggestion} |`);
    }
  }

  return lines.join("\n") + "\n";
}

function markdownEscape(value) {
  return String(value || "").replaceAll("`", "\\`");
}

function buildPullRequestReviewComment(finding) {
  const parts = [
    `**Documentation review (${finding.source || "review"}:${finding.category || "general"}, ${finding.severity})**`
  ];

  if (finding.message) parts.push(markdownEscape(finding.message));
  if (finding.suggestion) parts.push(`Suggestion: ${markdownEscape(finding.suggestion)}`);
  return parts.join("\n\n");
}

async function postPullRequestReview(reviewScope, findings, summary) {
  const shouldPost = (process.env.DOC_REVIEW_POST_PR_REVIEW || "true").toLowerCase() === "true";
  if (!shouldPost) return { skipped: true, reason: "DOC_REVIEW_POST_PR_REVIEW is disabled." };
  if (!reviewScope.enabled) return { skipped: true, reason: reviewScope.reason };
  if (!process.env.DOC_REVIEW_GITHUB_TOKEN && !process.env.GITHUB_TOKEN) {
    return { skipped: true, reason: "DOC_REVIEW_GITHUB_TOKEN or GITHUB_TOKEN is not set." };
  }

  const comments = findings
    .filter((finding) => finding.file && finding.line)
    .slice(0, 50)
    .map((finding) => ({
      path: finding.file,
      line: finding.line,
      side: "RIGHT",
      body: buildPullRequestReviewComment(finding)
    }));

  const payload = {
    event: "COMMENT",
    body: summary
  };
  if (comments.length > 0) payload.comments = comments;

  const response = await fetch(githubApiUrl(`/repos/${reviewScope.owner}/${reviewScope.repo}/pulls/${reviewScope.pullNumber}/reviews`), {
    method: "POST",
    headers: {
      ...githubApiHeaders(),
      "Content-Type": "application/json"
    },
    body: JSON.stringify(payload)
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(`GitHub pull request review request failed (${response.status}): ${body}`);
  }

  const review = await response.json();
  return { skipped: false, id: review.id, commentCount: comments.length };
}

async function main() {
  const bookSrc = process.env.DOC_REVIEW_BOOK_SRC || discoverBookSrc();
  const files = discoverMarkdownFiles(bookSrc);
  const styleGuide = fileExists(".github/doc-style-guide.md") ? readText(".github/doc-style-guide.md") : "";
  const reviewScope = await resolveReviewScope(files);

  let aiResult;
  try {
    aiResult = await runAiReview(files, styleGuide, reviewScope);
  } catch (error) {
    aiResult = { skipped: true, reason: error.message, findings: [] };
    console.log(`::warning title=${commandEscape("docs:ai-review")}::${commandEscape(`AI review failed: ${error.message}`)}`);
  }

  const allFindings = aiResult.findings;
  const findings = filterFindingsToReviewScope(allFindings, reviewScope);
  const excludedFindingCount = allFindings.length - findings.length;
  const summary = summarize(findings, aiResult, files, reviewScope, excludedFindingCount);

  let pullRequestReview;
  try {
    pullRequestReview = await postPullRequestReview(reviewScope, findings, summary);
  } catch (error) {
    pullRequestReview = { skipped: true, reason: error.message };
    console.log(`::warning title=${commandEscape("docs:pr-review")}::${commandEscape(`Could not post pull request review: ${error.message}`)}`);
  }

  const report = {
    generatedAt: new Date().toISOString(),
    bookSrc,
    files,
    reviewScope: serializeReviewScope(reviewScope),
    pullRequestReview,
    ai: aiResult.skipped ? { skipped: true, reason: aiResult.reason } : { skipped: false, provider: aiResult.provider, model: aiResult.model },
    excludedFindingCount,
    findings
  };

  fs.writeFileSync(path.join(repoRoot, reportPath), JSON.stringify(report, null, 2) + "\n");
  fs.writeFileSync(path.join(repoRoot, summaryPath), summary);

  if (process.env.GITHUB_STEP_SUMMARY) {
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
  }

  annotate(findings);

  const blockingFindings = findings.filter((finding) => finding.severity === "error" && finding.confidence >= failConfidence && (finding.source !== "ai" || failOnAiError));
  if (failOnError && blockingFindings.length > 0) {
    console.error(`Documentation review failed with ${blockingFindings.length} high-confidence error(s).`);
    process.exit(1);
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(2);
});
