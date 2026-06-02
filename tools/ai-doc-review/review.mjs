#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const repoRoot = process.cwd();
const reportPath = process.env.DOC_REVIEW_REPORT_PATH || "doc-review-report.json";
const summaryPath = process.env.DOC_REVIEW_SUMMARY_PATH || "doc-review-summary.md";
const failOnError = (process.env.DOC_REVIEW_FAIL_ON_ERROR || "true").toLowerCase() === "true";
const failConfidence = Number(process.env.DOC_REVIEW_FAIL_CONFIDENCE || "0.85");
const maxBookChars = Number(process.env.DOC_REVIEW_MAX_BOOK_CHARS || "120000");

function readText(filePath) {
  return fs.readFileSync(path.join(repoRoot, filePath), "utf8");
}

function readJson(filePath) {
  return JSON.parse(readText(filePath));
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
    const normalized = file.replaceAll(path.sep, "/");
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
      const linked = path.normalize(path.join(bookSrc, match[1])).replaceAll(path.sep, "/");
      add(linked);
    }
  }

  for (const file of walkMarkdown(bookSrc)) add(file);
  return ordered;
}

function spaces(length) {
  return " ".repeat(Math.max(0, length));
}

function replaceRangeWithSpaces(line, start, end) {
  return line.slice(0, start) + spaces(end - start) + line.slice(end);
}

function sanitizeMarkdownLine(line, state) {
  const fence = line.match(/^\s*(```|~~~)/);
  if (fence) {
    state.inFence = !state.inFence;
    return spaces(line.length);
  }
  if (state.inFence) return spaces(line.length);

  let sanitized = line;

  // Reference link definitions are metadata, not prose.
  if (/^\s*\[[^\]]+\]:\s+\S+/.test(sanitized)) return spaces(line.length);

  // Markdown link destinations: keep visible link text, hide URLs/paths.
  sanitized = sanitized.replace(/\]\(([^)]+)\)/g, (match, destination) => "](" + spaces(destination.length) + ")");

  // Bare URLs.
  sanitized = sanitized.replace(/https?:\/\/\S+/g, (match) => spaces(match.length));

  // Inline code spans.
  sanitized = sanitized.replace(/`[^`]*`/g, (match) => spaces(match.length));

  // HTML tags.
  sanitized = sanitized.replace(/<[^>]+>/g, (match) => spaces(match.length));

  return sanitized;
}

function runDeterministicChecks(files, terms) {
  const findings = [];

  for (const file of files) {
    const lines = readText(file).split(/\r?\n/);
    const state = { inFence: false };

    lines.forEach((line, index) => {
      const sanitized = sanitizeMarkdownLine(line, state);
      for (const rule of terms.deterministicChecks || []) {
        for (const pattern of rule.patterns || []) {
          const regex = new RegExp(pattern, "g");
          for (const match of sanitized.matchAll(regex)) {
            const matchedText = line.slice(match.index, match.index + match[0].length);
            findings.push({
              source: "deterministic",
              category: "terminology",
              ruleId: rule.id,
              severity: rule.severity || "warning",
              confidence: 1,
              file,
              line: index + 1,
              quote: matchedText,
              message: rule.message || `Use '${rule.preferred}' instead of '${matchedText}'.`,
              suggestion: rule.preferred ? `Use ${rule.preferred}.` : undefined
            });
          }
        }
      }
    });
  }

  return findings;
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

function normalizeAiFinding(finding) {
  const severity = ["suggestion", "warning", "error"].includes(finding.severity) ? finding.severity : "warning";
  const confidence = typeof finding.confidence === "number" ? Math.max(0, Math.min(1, finding.confidence)) : 0.75;
  return {
    source: "ai",
    category: finding.category || "consistency",
    ruleId: finding.ruleId || undefined,
    severity,
    confidence,
    file: finding.file,
    line: Number.isInteger(finding.line) ? finding.line : undefined,
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

async function runAiReview(files, terms) {
  const aiConfig = resolveAiConfig();
  if (!aiConfig) {
    return { skipped: true, reason: "AI_DOC_REVIEW_API_KEY, GITHUB_TOKEN, or OPENAI_API_KEY is not set.", findings: [] };
  }

  const styleGuide = fileExists(".github/doc-style-guide.md") ? readText(".github/doc-style-guide.md") : "";
  const bookPayload = buildBookPayload(files);
  const { apiKey, model, baseUrl, provider } = aiConfig;

  const systemPrompt = `You are a meticulous technical editor for an mdBook about Embedded Rust on Espressif hardware.
Review the whole book as one coherent document. Check spelling, grammar, terminology consistency, formatting consistency, voice/tone consistency, and cross-chapter continuity.
Return only actionable findings. Do not rewrite whole sections. Do not flag code examples, URLs, commands, or exact package/repository names unless the surrounding prose is wrong.
Return strict JSON with this shape: {"findings":[{"category":"spelling|grammar|terminology|formatting|consistency|voice|continuity","severity":"suggestion|warning|error","confidence":0.0,"file":"path","line":1,"quote":"exact text","message":"why this matters","suggestion":"specific replacement or action"}]}.
Use severity "error" only for high-confidence factual style violations or spelling errors. Limit output to the 50 most useful findings.`;

  const userPrompt = `Style guide:\n${styleGuide}\n\nCanonical terms and deterministic rules:\n${JSON.stringify(terms, null, 2)}\n\nBook files in reading order:\n${bookPayload}`;

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
  const findings = Array.isArray(parsed.findings) ? parsed.findings.map(normalizeAiFinding) : [];
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
    if (finding.category) properties.push(`title=${commandEscape(`docs:${finding.category}`)}`);
    const propertyText = properties.length > 0 ? ` ${properties.join(",")}` : "";
    const message = `${finding.message || "Documentation review finding"}${finding.suggestion ? ` Suggestion: ${finding.suggestion}` : ""}`;
    console.log(`::${level}${propertyText}::${commandEscape(message)}`);
  }
}

function summarize(findings, aiResult, files) {
  const counts = findings.reduce((acc, finding) => {
    acc[finding.severity] = (acc[finding.severity] || 0) + 1;
    return acc;
  }, {});

  const lines = [];
  lines.push("# Documentation review");
  lines.push("");
  lines.push(`Reviewed ${files.length} Markdown files.`);
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

async function main() {
  const bookSrc = process.env.DOC_REVIEW_BOOK_SRC || discoverBookSrc();
  const files = discoverMarkdownFiles(bookSrc);
  const terms = readJson(".github/doc-terms.json");

  const deterministicFindings = runDeterministicChecks(files, terms);
  let aiResult;
  try {
    aiResult = await runAiReview(files, terms);
  } catch (error) {
    aiResult = { skipped: true, reason: error.message, findings: [] };
    console.log(`::warning title=${commandEscape("docs:ai-review")}::${commandEscape(`AI review failed: ${error.message}`)}`);
  }

  const findings = [...deterministicFindings, ...aiResult.findings];
  const report = {
    generatedAt: new Date().toISOString(),
    bookSrc,
    files,
    ai: aiResult.skipped ? { skipped: true, reason: aiResult.reason } : { skipped: false, provider: aiResult.provider, model: aiResult.model },
    findings
  };

  fs.writeFileSync(path.join(repoRoot, reportPath), JSON.stringify(report, null, 2) + "\n");
  const summary = summarize(findings, aiResult, files);
  fs.writeFileSync(path.join(repoRoot, summaryPath), summary);

  if (process.env.GITHUB_STEP_SUMMARY) {
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
  }

  annotate(findings);

  const blockingFindings = findings.filter((finding) => finding.severity === "error" && finding.confidence >= failConfidence);
  if (failOnError && blockingFindings.length > 0) {
    console.error(`Documentation review failed with ${blockingFindings.length} high-confidence error(s).`);
    process.exit(1);
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(2);
});
