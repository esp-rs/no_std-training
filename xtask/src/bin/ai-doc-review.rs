use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use chat_rs::{
    ChatBuilder,
    completions::ChatCompletionsBuilder,
    parts,
    types::{messages, messages::content, options::ChatOptions},
};
use clap::Parser;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Parser)]
struct Args {
    /// Review all Markdown files on pull request runs instead of only changed lines.
    #[arg(long)]
    check_all: bool,
}

#[derive(Debug, Clone)]
struct Config {
    repo_root: PathBuf,
    report_path: String,
    summary_path: String,
    fail_on_error: bool,
    fail_on_ai_error: bool,
    fail_confidence: f64,
    max_book_chars: usize,
    check_all: bool,
}

#[derive(Debug, Clone, Default)]
struct ReviewScope {
    enabled: bool,
    reason: String,
    owner: Option<String>,
    repo: Option<String>,
    pull_number: Option<u64>,
    changed_lines_by_file: BTreeMap<String, BTreeSet<u64>>,
    limit_to_changed_lines: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Severity {
    Suggestion,
    Warning,
    Error,
}

impl Severity {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Suggestion => "suggestion",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Finding {
    source: String,
    category: String,
    #[serde(rename = "ruleId", skip_serializing_if = "Option::is_none")]
    rule_id: Option<String>,
    severity: Severity,
    confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum AiSeverity {
    Suggestion,
    Warning,
    Error,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct AiFinding {
    category: Option<String>,
    severity: Option<AiSeverity>,
    confidence: Option<f64>,
    file: Option<String>,
    line: Option<u64>,
    quote: Option<String>,
    message: Option<String>,
    suggestion: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct AiReview {
    findings: Vec<AiFinding>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiReport {
    skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Clone)]
struct AiResult {
    skipped: bool,
    reason: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    findings: Vec<Finding>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestReviewReport {
    skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct GitHubEvent {
    number: Option<u64>,
    repository: Option<GitHubRepository>,
    pull_request: Option<GitHubPullRequest>,
}

#[derive(Debug, Deserialize)]
struct GitHubRepository {
    full_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequest {
    number: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GitHubPullFile {
    filename: String,
    patch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubReviewResponse {
    id: u64,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error:?}");
        std::process::exit(2);
    }
}

async fn run() -> Result<()> {
    let args = Args::parse();
    let config = Config {
        repo_root: env::current_dir().context("failed to determine current directory")?,
        report_path: env::var("DOC_REVIEW_REPORT_PATH")
            .unwrap_or_else(|_| "doc-review-report.json".to_owned()),
        summary_path: env::var("DOC_REVIEW_SUMMARY_PATH")
            .unwrap_or_else(|_| "doc-review-summary.md".to_owned()),
        fail_on_error: env_bool("DOC_REVIEW_FAIL_ON_ERROR", true),
        fail_on_ai_error: env_bool("DOC_REVIEW_FAIL_ON_AI_ERROR", false),
        fail_confidence: env_f64("DOC_REVIEW_FAIL_CONFIDENCE", 0.85),
        max_book_chars: env_usize("DOC_REVIEW_MAX_BOOK_CHARS", 120_000),
        check_all: args.check_all || env_bool("DOC_REVIEW_CHECK_ALL", false),
    };

    let book_src = env::var("DOC_REVIEW_BOOK_SRC").unwrap_or_else(|_| discover_book_src(&config));
    let files = discover_markdown_files(&config, &book_src)?;
    let style_guide = read_optional(&config, ".github/doc-style-guide.md")?;
    let review_scope = resolve_review_scope(&files, config.check_all).await?;

    let mut ai_result = match run_ai_review(&config, &files, &style_guide, &review_scope).await {
        Ok(result) => result,
        Err(error) => {
            println!(
                "::warning title={}::{}",
                command_escape("docs:ai-review"),
                command_escape(format!("AI review failed: {error}"))
            );
            AiResult {
                skipped: true,
                reason: Some(error.to_string()),
                provider: None,
                model: None,
                findings: Vec::new(),
            }
        }
    };

    let all_finding_count = ai_result.findings.len();
    ai_result.findings = filter_findings_to_review_scope(ai_result.findings, &review_scope);
    let excluded_finding_count = all_finding_count - ai_result.findings.len();
    let summary = summarize(
        &ai_result.findings,
        &ai_result,
        &files,
        &review_scope,
        excluded_finding_count,
    );

    let pull_request_review =
        match post_pull_request_review(&review_scope, &ai_result.findings, &summary).await {
            Ok(result) => result,
            Err(error) => {
                println!(
                    "::warning title={}::{}",
                    command_escape("docs:pr-review"),
                    command_escape(format!("Could not post pull request review: {error}"))
                );
                PullRequestReviewReport {
                    skipped: true,
                    reason: Some(error.to_string()),
                    id: None,
                    comment_count: None,
                }
            }
        };

    let report = json!({
        "generatedAt": generated_at_timestamp(),
        "bookSrc": book_src,
        "files": files,
        "reviewScope": serialize_review_scope(&review_scope),
        "pullRequestReview": pull_request_review,
        "ai": AiReport {
            skipped: ai_result.skipped,
            reason: ai_result.reason.clone(),
            provider: ai_result.provider.clone(),
            model: ai_result.model.clone(),
        },
        "excludedFindingCount": excluded_finding_count,
        "findings": ai_result.findings.clone(),
    });

    fs::write(
        config.repo_root.join(&config.report_path),
        serde_json::to_string_pretty(&report)? + "\n",
    )?;
    fs::write(config.repo_root.join(&config.summary_path), &summary)?;

    if let Ok(step_summary) = env::var("GITHUB_STEP_SUMMARY") {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(step_summary)?
            .write_all(summary.as_bytes())?;
    }

    annotate(&ai_result.findings);

    let blocking = ai_result
        .findings
        .iter()
        .filter(|finding| {
            matches!(finding.severity, Severity::Error)
                && finding.confidence >= config.fail_confidence
                && (finding.source != "ai" || config.fail_on_ai_error)
        })
        .count();

    if config.fail_on_error && blocking > 0 {
        eprintln!("Documentation review failed with {blocking} high-confidence error(s).");
        std::process::exit(1);
    }

    Ok(())
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn read_text(config: &Config, file_path: impl AsRef<Path>) -> Result<String> {
    fs::read_to_string(config.repo_root.join(file_path)).context("failed to read file")
}

fn read_optional(config: &Config, file_path: impl AsRef<Path>) -> Result<String> {
    let path = config.repo_root.join(file_path);
    if path.exists() {
        fs::read_to_string(path).context("failed to read optional file")
    } else {
        Ok(String::new())
    }
}

fn file_exists(config: &Config, file_path: impl AsRef<Path>) -> bool {
    config.repo_root.join(file_path).exists()
}

fn discover_book_src(config: &Config) -> String {
    if !file_exists(config, "book.toml") {
        return "training".to_owned();
    }

    let Ok(book_toml) = read_text(config, "book.toml") else {
        return "training".to_owned();
    };

    for line in book_toml.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("src") {
            let value = value.trim_start();
            if let Some(value) = value.strip_prefix('=') {
                let value = value.trim().trim_matches(['\'', '"']);
                if !value.is_empty() {
                    return value.to_owned();
                }
            }
        }
    }

    "training".to_owned()
}

fn discover_markdown_files(config: &Config, book_src: &str) -> Result<Vec<String>> {
    let summary = normalize_repo_path(Path::new(book_src).join("SUMMARY.md").to_string_lossy());
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();

    add_markdown_file(config, &mut ordered, &mut seen, summary);

    if file_exists(config, Path::new(book_src).join("SUMMARY.md")) {
        let summary_text = read_text(config, Path::new(book_src).join("SUMMARY.md"))?;
        for linked in extract_summary_markdown_links(&summary_text) {
            let linked = normalize_repo_path(Path::new(book_src).join(linked).to_string_lossy());
            add_markdown_file(config, &mut ordered, &mut seen, linked);
        }
    }

    for file in walk_markdown(config, book_src)? {
        add_markdown_file(config, &mut ordered, &mut seen, file);
    }

    Ok(ordered)
}

fn add_markdown_file(
    config: &Config,
    ordered: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
    file: String,
) {
    let normalized = normalize_repo_path(file);
    if !seen.contains(&normalized) && file_exists(config, &normalized) {
        seen.insert(normalized.clone());
        ordered.push(normalized);
    }
}

fn extract_summary_markdown_links(summary: &str) -> Vec<String> {
    let mut links = Vec::new();
    for segment in summary.split("](").skip(1) {
        let Some(destination) = segment.split(')').next() else {
            continue;
        };
        let destination = destination.split('#').next().unwrap_or(destination);
        if destination.ends_with(".md") {
            links.push(destination.to_owned());
        }
    }
    links
}

fn walk_markdown(config: &Config, dir: &str) -> Result<Vec<String>> {
    let mut results = Vec::new();
    let absolute = config.repo_root.join(dir);
    if !absolute.exists() {
        return Ok(results);
    }

    for entry in fs::read_dir(&absolute)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let relative =
                normalize_repo_path(path.strip_prefix(&config.repo_root)?.to_string_lossy());
            results.extend(walk_markdown(config, &relative)?);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            results.push(normalize_repo_path(
                path.strip_prefix(&config.repo_root)?.to_string_lossy(),
            ));
        }
    }

    results.sort();
    Ok(results)
}

fn normalize_repo_path(file_path: impl AsRef<str>) -> String {
    let replaced = file_path.as_ref().replace('\\', "/");
    let mut parts = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

fn parse_changed_lines_from_patch(patch: &str) -> BTreeSet<u64> {
    let mut changed_lines = BTreeSet::new();
    let mut new_line = 0u64;

    for line in patch.lines() {
        if let Some(start) = parse_hunk_new_start(line) {
            new_line = start;
            continue;
        }

        if line.starts_with("+++") {
            continue;
        }
        if line.starts_with('+') {
            changed_lines.insert(new_line);
            new_line += 1;
        } else if line.starts_with('-') {
            continue;
        } else if line.starts_with(' ') {
            new_line += 1;
        }
    }

    changed_lines
}

fn parse_hunk_new_start(line: &str) -> Option<u64> {
    let rest = line.strip_prefix("@@ -")?;
    let plus = rest.find(" +")?;
    let rest = &rest[plus + 2..];
    let end = rest.find([',', ' ']).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

async fn resolve_review_scope(files: &[String], check_all: bool) -> Result<ReviewScope> {
    let event_name = env::var("GITHUB_EVENT_NAME").unwrap_or_default();
    if event_name != "pull_request" && event_name != "pull_request_target" {
        return Ok(ReviewScope {
            enabled: false,
            reason: "No pull request event detected.".to_owned(),
            ..ReviewScope::default()
        });
    }

    let event_path = env::var("GITHUB_EVENT_PATH")
        .context("Cannot determine PR diff: GITHUB_EVENT_PATH is not set.")?;
    let event: GitHubEvent = serde_json::from_str(&fs::read_to_string(event_path)?)?;
    let pull_number = event
        .pull_request
        .and_then(|pull| pull.number)
        .or(event.number)
        .context("Cannot determine PR diff: pull request number is missing.")?;
    let repository = event
        .repository
        .and_then(|repository| repository.full_name)
        .or_else(|| env::var("GITHUB_REPOSITORY").ok())
        .context("Cannot determine PR diff: repository is missing.")?;
    let (owner, repo) = repository
        .split_once('/')
        .context("Cannot determine PR diff: repository is invalid.")?;

    if check_all {
        return Ok(ReviewScope {
            enabled: true,
            reason: format!("Pull request #{pull_number}; full-book review is enabled"),
            owner: Some(owner.to_owned()),
            repo: Some(repo.to_owned()),
            pull_number: Some(pull_number),
            changed_lines_by_file: BTreeMap::new(),
            limit_to_changed_lines: false,
        });
    }

    let reviewed_files: BTreeSet<_> = files.iter().cloned().collect();
    let pr_files = fetch_pull_request_files(owner, repo, pull_number).await?;
    let mut changed_lines_by_file = BTreeMap::new();

    for pr_file in pr_files {
        let file = normalize_repo_path(pr_file.filename);
        if !reviewed_files.contains(&file) {
            continue;
        }
        let Some(patch) = pr_file.patch else {
            continue;
        };
        let changed_lines = parse_changed_lines_from_patch(&patch);
        if !changed_lines.is_empty() {
            changed_lines_by_file.insert(file, changed_lines);
        }
    }

    Ok(ReviewScope {
        enabled: true,
        reason: format!("Pull request #{pull_number}"),
        owner: Some(owner.to_owned()),
        repo: Some(repo.to_owned()),
        pull_number: Some(pull_number),
        changed_lines_by_file,
        limit_to_changed_lines: true,
    })
}

async fn fetch_pull_request_files(
    owner: &str,
    repo: &str,
    pull_number: u64,
) -> Result<Vec<GitHubPullFile>> {
    let client = reqwest::Client::new();
    let mut files = Vec::new();

    for page in 1.. {
        let url = github_api_url(&format!(
            "/repos/{owner}/{repo}/pulls/{pull_number}/files?per_page=100&page={page}"
        ));
        let response = client.get(url).headers(github_headers()?).send().await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "GitHub API request failed ({}): {}",
                response.status(),
                response.text().await?
            ));
        }
        let page_files: Vec<GitHubPullFile> = response.json().await?;
        let done = page_files.len() < 100;
        files.extend(page_files);
        if done {
            break;
        }
    }

    Ok(files)
}

fn github_headers() -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("no-std-training-ai-doc-review"),
    );
    if let Some(token) =
        env_nonempty("DOC_REVIEW_GITHUB_TOKEN").or_else(|| env_nonempty("GITHUB_TOKEN"))
    {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))?,
        );
    }
    Ok(headers)
}

fn github_api_url(pathname: &str) -> String {
    let base = env::var("GITHUB_API_URL").unwrap_or_else(|_| "https://api.github.com".to_owned());
    format!("{}{}", base.trim_end_matches('/'), pathname)
}

fn compact_line_numbers(lines: &BTreeSet<u64>) -> String {
    let mut ranges = Vec::new();
    let mut iter = lines.iter().copied().peekable();

    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek().is_some_and(|next| *next == end + 1) {
            end = iter.next().unwrap();
        }
        if start == end {
            ranges.push(start.to_string());
        } else {
            ranges.push(format!("{start}-{end}"));
        }
    }

    ranges.join(", ")
}

fn count_review_scope_lines(review_scope: &ReviewScope) -> usize {
    review_scope
        .changed_lines_by_file
        .values()
        .map(BTreeSet::len)
        .sum()
}

fn format_review_scope(review_scope: &ReviewScope) -> String {
    if !review_scope.enabled {
        return format!(
            "No PR diff scope detected ({}); findings may cover any reviewed file.",
            review_scope.reason
        );
    }
    if !review_scope.limit_to_changed_lines {
        return "Review every included Markdown file. Do not restrict findings to PR changed lines."
            .to_owned();
    }
    if review_scope.changed_lines_by_file.is_empty() {
        return "The PR does not add or modify any lines in the reviewed Markdown files. Return {\"findings\":[]}.".to_owned();
    }

    let entries = review_scope
        .changed_lines_by_file
        .iter()
        .map(|(file, lines)| format!("- {file}: {}", compact_line_numbers(lines)))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Only return findings whose file and line are in these PR changed lines:\n{entries}")
}

fn finding_matches_review_scope(finding: &Finding, review_scope: &ReviewScope) -> bool {
    if !review_scope.limit_to_changed_lines {
        return true;
    }
    let Some(file) = finding.file.as_ref().map(normalize_repo_path) else {
        return false;
    };
    let Some(line) = finding.line else {
        return false;
    };
    review_scope
        .changed_lines_by_file
        .get(&file)
        .is_some_and(|lines| lines.contains(&line))
}

fn filter_findings_to_review_scope(
    findings: Vec<Finding>,
    review_scope: &ReviewScope,
) -> Vec<Finding> {
    findings
        .into_iter()
        .filter(|finding| finding_matches_review_scope(finding, review_scope))
        .collect()
}

fn serialize_review_scope(review_scope: &ReviewScope) -> serde_json::Value {
    if !review_scope.enabled {
        return json!({ "enabled": false, "reason": review_scope.reason });
    }

    let changed_lines: BTreeMap<_, _> = review_scope
        .changed_lines_by_file
        .iter()
        .map(|(file, lines)| (file, lines.iter().copied().collect::<Vec<_>>()))
        .collect();

    json!({
        "enabled": true,
        "reason": review_scope.reason,
        "repository": format!("{}/{}", review_scope.owner.as_deref().unwrap_or_default(), review_scope.repo.as_deref().unwrap_or_default()),
        "pullNumber": review_scope.pull_number,
        "limitToChangedLines": review_scope.limit_to_changed_lines,
        "changedLines": changed_lines,
    })
}

fn build_book_payload(config: &Config, files: &[String]) -> Result<String> {
    let mut chunks = Vec::new();
    let mut total = 0usize;

    for file in files {
        let text = read_text(config, file)?;
        let chunk = format!("<file path=\"{file}\">\n{text}\n</file>");
        if total + chunk.len() > config.max_book_chars {
            chunks.push(format!(
                "\n<!-- Skipped {file}: book payload exceeded {} characters. -->",
                config.max_book_chars
            ));
            continue;
        }
        total += chunk.len();
        chunks.push(chunk);
    }

    Ok(chunks.join("\n\n"))
}

async fn run_ai_review(
    config: &Config,
    files: &[String],
    style_guide: &str,
    review_scope: &ReviewScope,
) -> Result<AiResult> {
    let Some(ai_config) = resolve_ai_config() else {
        return Ok(AiResult {
            skipped: true,
            reason: Some(
                "AI_DOC_REVIEW_API_KEY, GITHUB_TOKEN, or OPENAI_API_KEY is not set.".to_owned(),
            ),
            provider: None,
            model: None,
            findings: Vec::new(),
        });
    };

    let book_payload = build_book_payload(config, files)?;
    let review_scope_text = format_review_scope(review_scope);

    let system_prompt = "You are a meticulous technical editor for an mdBook about Embedded Rust on Espressif hardware.\n\
Review the whole book as one coherent document. Check spelling, grammar, terminology consistency, formatting consistency, voice/tone consistency, and cross-chapter continuity.\n\
When a PR diff scope is provided, only report actionable findings on PR changed lines. You may use the unchanged surrounding content for context, but do not report findings outside that scope.\n\
Return only actionable findings where the quoted text should be changed. Do not report correct usage, and never use suggestions like \"No change needed\".\n\
Omit findings when the suggested replacement is identical to the quoted text.\n\
Do not rewrite whole sections. Do not flag code examples, URLs, commands, fenced code blocks, inline code, Markdown link destinations, or exact package/repository names unless the surrounding prose is wrong.\n\
Do not invent terminology rules that are not present in the style guide.\n\
Return strict JSON with this shape: {\"findings\":[{\"category\":\"spelling|grammar|terminology|formatting|consistency|voice|continuity\",\"severity\":\"suggestion|warning|error\",\"confidence\":0.0,\"file\":\"path\",\"line\":1,\"quote\":\"exact text\",\"message\":\"why this matters\",\"suggestion\":\"specific replacement or action\"}]}.\n\
Use severity \"error\" only for high-confidence factual style violations explicitly covered by the style guide, or clear spelling errors. Limit output to the 50 most useful findings.";

    let user_prompt = format!(
        "Style guide:\n{style_guide}\n\nPR diff scope:\n{review_scope_text}\n\nBook files in reading order:\n{book_payload}"
    );

    let mut builder = ChatCompletionsBuilder::new()
        .with_base_url(&ai_config.base_url)
        .with_model(&ai_config.model);
    if let Some(api_key) = &ai_config.api_key {
        builder = builder.with_api_key(api_key);
    }

    let client = builder.build();
    let mut options = ChatOptions::default();
    options.temperature = Some(0.1);
    options.max_tokens = Some(env_usize("DOC_REVIEW_MAX_TOKENS", 4000) as u32);

    let mut chat = ChatBuilder::new()
        .with_structured_output::<AiReview>()
        .with_model(client)
        .with_options(options)
        .build();

    let mut messages = messages::Messages::default();
    messages.push(content::from_system(parts![system_prompt]));
    messages.push(content::from_user(parts![user_prompt]));

    let response = chat
        .complete(&mut messages)
        .await
        .map_err(|error| anyhow!("{} API request failed: {}", ai_config.provider, error.err))?
        .expect_complete();
    let findings = response
        .content
        .findings
        .into_iter()
        .filter_map(|finding| normalize_ai_finding(finding, config.fail_on_ai_error))
        .map(|finding| reconcile_finding_location(config, finding))
        .collect();

    Ok(AiResult {
        skipped: false,
        reason: None,
        provider: Some(ai_config.provider),
        model: Some(ai_config.model),
        findings,
    })
}

#[derive(Debug, Clone)]
struct AiConfig {
    api_key: Option<String>,
    model: String,
    base_url: String,
    provider: String,
}

fn resolve_ai_config() -> Option<AiConfig> {
    let configured_base_url =
        env_nonempty("AI_DOC_REVIEW_BASE_URL").or_else(|| env_nonempty("OPENAI_BASE_URL"));
    let api_key = env_nonempty("AI_DOC_REVIEW_API_KEY")
        .or_else(|| env_nonempty("GITHUB_TOKEN"))
        .or_else(|| env_nonempty("OPENAI_API_KEY"));

    api_key.as_ref()?;

    let uses_github_models = configured_base_url
        .as_deref()
        .is_some_and(|url| url.contains("models.github.ai"))
        || (configured_base_url.is_none()
            && env_nonempty("GITHUB_TOKEN").is_some()
            && env_nonempty("OPENAI_API_KEY").is_none());
    let default_base_url = if uses_github_models {
        "https://models.github.ai/inference"
    } else {
        "https://api.openai.com/v1"
    };
    let default_model = if uses_github_models {
        "openai/gpt-4.1-mini"
    } else {
        "gpt-4.1-mini"
    };

    Some(AiConfig {
        api_key,
        model: env_nonempty("AI_DOC_REVIEW_MODEL")
            .or_else(|| env_nonempty("OPENAI_MODEL"))
            .unwrap_or_else(|| default_model.to_owned()),
        base_url: configured_base_url
            .unwrap_or_else(|| default_base_url.to_owned())
            .trim_end_matches('/')
            .to_owned(),
        provider: if uses_github_models {
            "GitHub Models"
        } else {
            "OpenAI-compatible"
        }
        .to_owned(),
    })
}

fn normalize_ai_finding(finding: AiFinding, fail_on_ai_error: bool) -> Option<Finding> {
    let message = finding.message.unwrap_or_default();
    let suggestion = finding.suggestion.unwrap_or_default();
    let quote = finding.quote.unwrap_or_default();
    let review_text = format!("{message} {suggestion}");

    let non_actionable = [
        "no change needed",
        "no further action",
        "correct use",
        "correctly done",
        "correct phrasing",
        "correct here",
        "this is correct",
        "correct as is",
    ];
    if non_actionable
        .iter()
        .any(|phrase| review_text.to_lowercase().contains(phrase))
    {
        return None;
    }

    if !suggestion.is_empty()
        && normalize_finding_text(&suggestion) == normalize_finding_text(&quote)
    {
        return None;
    }

    let mut severity = match finding.severity.unwrap_or(AiSeverity::Warning) {
        AiSeverity::Suggestion => Severity::Suggestion,
        AiSeverity::Warning => Severity::Warning,
        AiSeverity::Error => Severity::Error,
    };
    if matches!(severity, Severity::Error) && !fail_on_ai_error {
        severity = Severity::Warning;
    }

    Some(Finding {
        source: "ai".to_owned(),
        category: finding.category.unwrap_or_else(|| "consistency".to_owned()),
        rule_id: None,
        severity,
        confidence: finding.confidence.unwrap_or(0.75).clamp(0.0, 1.0),
        file: finding.file.map(normalize_repo_path),
        line: finding.line.filter(|line| *line > 0),
        quote: if quote.is_empty() { None } else { Some(quote) },
        message: if message.is_empty() {
            None
        } else {
            Some(message)
        },
        suggestion: if suggestion.is_empty() {
            None
        } else {
            Some(suggestion)
        },
    })
}

fn reconcile_finding_location(config: &Config, mut finding: Finding) -> Finding {
    let Some(file) = finding.file.as_deref() else {
        return finding;
    };
    let Some(quote) = finding.quote.as_deref() else {
        return finding;
    };
    let Ok(text) = read_text(config, file) else {
        return finding;
    };
    let Some(line) = locate_quote_line(&text, quote) else {
        return finding;
    };

    finding.line = Some(line);
    finding
}

fn locate_quote_line(text: &str, quote: &str) -> Option<u64> {
    let quote = quote.trim();
    if quote.is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    let mut search_start = 0usize;
    while let Some(relative_index) = text[search_start..].find(quote) {
        let index = search_start + relative_index;
        matches.push(line_number_at_byte(text, index));
        search_start = index + quote.len();
    }
    matches.sort_unstable();
    matches.dedup();
    if matches.len() == 1 {
        return matches.first().copied();
    }

    let normalized_quote = normalize_quote_whitespace(quote);
    let mut line_matches = text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            (normalize_quote_whitespace(line) == normalized_quote).then_some(index as u64 + 1)
        })
        .collect::<Vec<_>>();
    line_matches.sort_unstable();
    line_matches.dedup();
    if line_matches.len() == 1 {
        line_matches.first().copied()
    } else {
        None
    }
}

fn line_number_at_byte(text: &str, byte_index: usize) -> u64 {
    text[..byte_index]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count() as u64
        + 1
}

fn normalize_quote_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_finding_text(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("Suggestion:")
        .trim_start_matches("Change to:")
        .trim_start_matches(['-', '*', ' '])
        .trim_matches(['\'', '"'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn command_escape(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn annotate(findings: &[Finding]) {
    for finding in findings {
        let level = if matches!(finding.severity, Severity::Error) {
            "error"
        } else {
            "warning"
        };
        let mut properties = Vec::new();
        if let Some(file) = &finding.file {
            properties.push(format!("file={}", command_escape(file)));
        }
        if let Some(line) = finding.line {
            properties.push(format!("line={}", command_escape(line.to_string())));
        }
        properties.push(format!(
            "title={}",
            command_escape(format!("docs:{}:{}", finding.source, finding.category))
        ));
        let property_text = format!(" {}", properties.join(","));
        let message = finding
            .message
            .as_deref()
            .unwrap_or("Documentation review finding");
        let suggestion = finding
            .suggestion
            .as_ref()
            .map(|suggestion| format!(" Suggestion: {suggestion}"))
            .unwrap_or_default();
        println!(
            "::{level}{property_text}::{}",
            command_escape(format!("{message}{suggestion}"))
        );
    }
}

fn summarize(
    findings: &[Finding],
    ai_result: &AiResult,
    files: &[String],
    review_scope: &ReviewScope,
    excluded_finding_count: usize,
) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for finding in findings {
        *counts.entry(finding.severity.as_str()).or_default() += 1;
    }

    let mut lines = Vec::new();
    lines.push("# Documentation review".to_owned());
    lines.push(String::new());
    lines.push(format!("Reviewed {} Markdown files.", files.len()));
    if review_scope.enabled {
        if review_scope.limit_to_changed_lines {
            lines.push(format!(
                "Actionable scope: {} changed line(s) in {} Markdown file(s) from {}.",
                count_review_scope_lines(review_scope),
                review_scope.changed_lines_by_file.len(),
                review_scope.reason
            ));
        } else {
            lines.push(format!(
                "Actionable scope: all reviewed Markdown files from {}.",
                review_scope.reason
            ));
        }
    }
    if excluded_finding_count > 0 {
        lines.push(format!(
            "Ignored {excluded_finding_count} finding(s) outside the PR diff."
        ));
    }
    lines.push(format!(
        "Findings: {} ({} errors, {} warnings, {} suggestions).",
        findings.len(),
        counts.get("error").copied().unwrap_or_default(),
        counts.get("warning").copied().unwrap_or_default(),
        counts.get("suggestion").copied().unwrap_or_default()
    ));
    if ai_result.skipped {
        lines.push(format!(
            "AI review skipped: {}",
            ai_result.reason.as_deref().unwrap_or("unknown reason")
        ));
    } else {
        lines.push(format!(
            "AI provider: {}",
            ai_result.provider.as_deref().unwrap_or("OpenAI-compatible")
        ));
        lines.push(format!(
            "AI model: {}",
            ai_result.model.as_deref().unwrap_or_default()
        ));
    }
    lines.push(String::new());

    if findings.is_empty() {
        lines.push("No documentation findings.".to_owned());
    } else {
        lines.push("| Severity | Source | File | Line | Category | Finding |".to_owned());
        lines.push("| --- | --- | --- | ---: | --- | --- |".to_owned());
        for finding in findings.iter().take(50) {
            let message = finding
                .message
                .as_deref()
                .unwrap_or_default()
                .replace('|', "\\|")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let suggestion = finding
                .suggestion
                .as_ref()
                .map(|suggestion| format!(" Suggested: {suggestion}"))
                .unwrap_or_default();
            lines.push(format!(
                "| {} | {} | {} | {} | {} | {}{} |",
                finding.severity.as_str(),
                finding.source,
                finding.file.as_deref().unwrap_or_default(),
                finding
                    .line
                    .map(|line| line.to_string())
                    .unwrap_or_default(),
                finding.category,
                message,
                suggestion
            ));
        }
    }

    lines.join("\n") + "\n"
}

fn markdown_escape(value: &str) -> String {
    value.replace('`', "\\`")
}

fn build_pull_request_review_comment(finding: &Finding) -> String {
    let mut parts = vec![format!(
        "**Documentation review ({}:{}, {})**",
        finding.source,
        finding.category,
        finding.severity.as_str()
    )];
    if let Some(message) = &finding.message {
        parts.push(markdown_escape(message));
    }
    if let Some(suggestion) = &finding.suggestion {
        parts.push(format!("Suggestion: {}", markdown_escape(suggestion)));
    }
    parts.join("\n\n")
}

async fn post_pull_request_review(
    review_scope: &ReviewScope,
    findings: &[Finding],
    summary: &str,
) -> Result<PullRequestReviewReport> {
    if !env_bool("DOC_REVIEW_POST_PR_REVIEW", true) {
        return Ok(PullRequestReviewReport {
            skipped: true,
            reason: Some("DOC_REVIEW_POST_PR_REVIEW is disabled.".to_owned()),
            id: None,
            comment_count: None,
        });
    }
    if !review_scope.enabled {
        return Ok(PullRequestReviewReport {
            skipped: true,
            reason: Some(review_scope.reason.clone()),
            id: None,
            comment_count: None,
        });
    }
    if env_nonempty("DOC_REVIEW_GITHUB_TOKEN").is_none() && env_nonempty("GITHUB_TOKEN").is_none() {
        return Ok(PullRequestReviewReport {
            skipped: true,
            reason: Some("DOC_REVIEW_GITHUB_TOKEN or GITHUB_TOKEN is not set.".to_owned()),
            id: None,
            comment_count: None,
        });
    }

    let comments = findings
        .iter()
        .filter(|finding| {
            review_scope.limit_to_changed_lines
                && finding_matches_review_scope(finding, review_scope)
        })
        .filter_map(|finding| {
            Some(json!({
                "path": finding.file.as_ref()?,
                "line": finding.line?,
                "side": "RIGHT",
                "body": build_pull_request_review_comment(finding),
            }))
        })
        .take(50)
        .collect::<Vec<_>>();

    let mut payload = json!({
        "event": "COMMENT",
        "body": summary,
    });
    if !comments.is_empty() {
        payload["comments"] = json!(comments);
    }

    let owner = review_scope.owner.as_deref().context("missing PR owner")?;
    let repo = review_scope
        .repo
        .as_deref()
        .context("missing PR repository")?;
    let pull_number = review_scope.pull_number.context("missing PR number")?;
    let url = github_api_url(&format!(
        "/repos/{owner}/{repo}/pulls/{pull_number}/reviews"
    ));
    let mut headers = github_headers()?;
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let response = reqwest::Client::new()
        .post(url)
        .headers(headers)
        .json(&payload)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub pull request review request failed ({}): {}",
            response.status(),
            response.text().await?
        ));
    }

    let review: GitHubReviewResponse = response.json().await?;
    Ok(PullRequestReviewReport {
        skipped: false,
        reason: None,
        id: Some(review.id),
        comment_count: Some(comments.len()),
    })
}

fn generated_at_timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_exact_quote_line() {
        let text = "# Project Overview\n\nIntro.\n\nMore intro.\n\n- First in [Project Setup](./project-setup.md), we will walk you through the process of initializing a project.\n";
        let quote = "- First in [Project Setup](./project-setup.md), we will walk you through the process of initializing a project.";

        assert_eq!(locate_quote_line(text, quote), Some(7));
    }
}

use std::io::Write;
