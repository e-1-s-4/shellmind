//! Prompt templates for the Ollama path.
//!
//! The output contract is deliberately simple and robust for small local
//! models:
//!
//! * the **first** fenced code block is the suggested command,
//! * any **further** fenced blocks are safer alternatives,
//! * the remaining prose is the explanation.
//!
//! Everything sent to the model passes through the redaction layer first
//! (see `AiEngine`).

pub const EXPLAIN_SYSTEM: &str = "\
You are shellmind, a concise terminal assistant embedded in the user's shell.
Explain shell commands briefly and practically. Use this structure:
1. One sentence about what the command does overall.
2. Short lines explaining each flag or subcommand that appears.
3. If a common pitfall exists, add a single warning line.
Never invent flags. Keep it under 12 lines. No markdown headers.";

pub const FIX_SYSTEM: &str = "\
You are shellmind, a terminal debugging assistant.
Given a failed command and its error output, suggest the smallest fix.
Respond with:
1. A fenced code block containing the corrected command.
2. One or two sentences explaining what was wrong.
3. If the fix could be destructive, add a fenced block with a safer alternative.
Never invent flags or files that were not mentioned.";

pub const GENERATE_SYSTEM: &str = "\
You are shellmind, a natural-language to shell-command translator.
The user describes what they want; you output a single shell command.
Rules:
- Respond with ONE fenced code block containing exactly the command, nothing else in the block.
- Prefer standard, portable commands. Respect the user's OS given in the context.
- After the block, add one sentence explaining the command.
- If the command is destructive, add a second fenced block with a safer alternative.
- Never use sudo unless the request clearly requires it.";

/// Build the user prompt for `explain`.
pub fn explain_user(command: &str, context_block: &str) -> String {
    format!(
        "Context:\n{context_block}\n\nExplain this command:\n\n```\n{command}\n```"
    )
}

/// Build the user prompt for `fix`.
pub fn fix_user(command: &str, error: &str, context_block: &str) -> String {
    format!(
        "Context:\n{context_block}\n\nFailed command:\n\n```\n{command}\n```\n\nError output:\n\n```\n{error}\n```"
    )
}

/// Build the user prompt for `generate`.
pub fn generate_user(query: &str, context_block: &str) -> String {
    format!("Context:\n{context_block}\n\nThe user wants: {query}")
}

/// Parsed LLM response.
#[derive(Debug, Default, Clone)]
pub struct LlmAnswer {
    pub command: Option<String>,
    pub alternatives: Vec<String>,
    pub explanation: String,
}

/// Parse an LLM answer according to the fenced-block contract.
pub fn parse_llm_answer(raw: &str) -> LlmAnswer {
    let mut answer = LlmAnswer::default();
    let mut blocks: Vec<String> = Vec::new();
    let mut rest = raw.to_string();
    // Extract fenced blocks (``` or ```bash).
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        // Skip language tag on the opening line.
        let line_end = after.find('\n').unwrap_or(after.len());
        let content_start = line_end + 1;
        if let Some(end) = after[content_start..].find("```") {
            let content = &after[content_start..content_start + end];
            let cleaned: String = content
                .lines()
                .map(|l| l.trim_start_matches('$').trim_start_matches('#').trim_start())
                .collect::<Vec<&str>>()
                .join("\n")
                .trim()
                .to_string();
            if !cleaned.is_empty() {
                blocks.push(cleaned);
            }
            let consumed = start + 3 + content_start + end + 3;
            rest = format!("{}{}", &rest[..start], &rest[consumed.min(rest.len())..]);
        } else {
            // Unterminated block: take the rest.
            let content = after[content_start..].trim();
            if !content.is_empty() {
                blocks.push(content.to_string());
            }
            rest = rest[..start].to_string();
            break;
        }
    }
    if let Some(first) = blocks.first() {
        answer.command = Some(first.clone());
        answer.alternatives = blocks[1..].to_vec();
    }
    answer.explanation = rest.trim().to_string();
    if answer.command.is_none() {
        // No fenced block: treat the first non-empty line as the command if
        // it looks like one, else keep everything as explanation.
        let first_line = raw.lines().find(|l| !l.trim().is_empty());
        if let Some(line) = first_line {
            let l = line.trim();
            if !l.ends_with('.') && !l.ends_with('?') && l.len() < 120 && !l.contains(' ') == false {
                // Heuristic: lines starting with a known command word.
                let first_word = l.split_whitespace().next().unwrap_or("");
                if !first_word.ends_with(':') && !l.starts_with("The ") && !l.starts_with("This ") {
                    answer.command = Some(l.to_string());
                    answer.explanation = raw
                        .lines()
                        .skip_while(|x| x.trim() != l)
                        .skip(1)
                        .collect::<Vec<&str>>()
                        .join("\n")
                        .trim()
                        .to_string();
                }
            }
        }
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_contract() {
        let raw = "Here you go:\n\n```bash\nfind . -type f -size +100M\n```\n\nThis finds files over 100MB.\n\nSafer:\n\n```\nfind . -type f -size +100M -print\n```";
        let a = parse_llm_answer(raw);
        assert_eq!(a.command.as_deref(), Some("find . -type f -size +100M"));
        assert_eq!(a.alternatives, vec!["find . -type f -size +100M -print"]);
        assert!(a.explanation.contains("finds files"));
    }

    #[test]
    fn parses_prompt_markers() {
        let raw = "```\n$ git push --set-upstream origin main\n```\nSets the upstream tracking branch.";
        let a = parse_llm_answer(raw);
        assert_eq!(a.command.as_deref(), Some("git push --set-upstream origin main"));
        assert!(a.explanation.contains("upstream"));
    }

    #[test]
    fn prompts_include_context() {
        let p = explain_user("tar -xzvf f.tar.gz", "os: linux\nshell: zsh");
        assert!(p.contains("os: linux"));
        assert!(p.contains("tar -xzvf"));
    }
}
