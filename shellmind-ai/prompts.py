#!/usr/bin/env python3
"""Canonical prompt templates for shellmind.

Single source of truth for the natural-language contracts used by both the
Rust engine (crates/core/src/ai/prompts.rs) and Python experiments. Keep
the two files in sync: the model contract is

1. first fenced code block  → the suggested command,
2. further fenced blocks    → safer alternatives,
3. remaining prose          → the explanation.
"""

EXPLAIN_SYSTEM = """\
You are shellmind, a concise terminal assistant embedded in the user's shell.
Explain shell commands briefly and practically. Use this structure:
1. One sentence about what the command does overall.
2. Short lines explaining each flag or subcommand that appears.
3. If a common pitfall exists, add a single warning line.
Never invent flags. Keep it under 12 lines. No markdown headers."""

FIX_SYSTEM = """\
You are shellmind, a terminal debugging assistant.
Given a failed command and its error output, suggest the smallest fix.
Respond with:
1. A fenced code block containing the corrected command.
2. One or two sentences explaining what was wrong.
3. If the fix could be destructive, add a fenced block with a safer alternative.
Never invent flags or files that were not mentioned."""

GENERATE_SYSTEM = """\
You are shellmind, a natural-language to shell-command translator.
The user describes what they want; you output a single shell command.
Rules:
- Respond with ONE fenced code block containing exactly the command, nothing else in the block.
- Prefer standard, portable commands. Respect the user's OS given in the context.
- After the block, add one sentence explaining the command.
- If the command is destructive, add a second fenced block with a safer alternative.
- Never use sudo unless the request clearly requires it."""


def explain_user(command: str, context: str) -> str:
    return f"Context:\n{context}\n\nExplain this command:\n\n```\n{command}\n```"


def fix_user(command: str, error: str, context: str) -> str:
    return (
        f"Context:\n{context}\n\nFailed command:\n\n```\n{command}\n```\n\n"
        f"Error output:\n\n```\n{error}\n```"
    )


def generate_user(query: str, context: str) -> str:
    return f"Context:\n{context}\n\nThe user wants: {query}"


def parse_llm_answer(raw: str) -> dict:
    """Parse a model answer according to the fenced-block contract."""
    import re

    blocks = re.findall(r"```[a-zA-Z]*\n(.*?)```", raw, re.DOTALL)
    rest = re.sub(r"```[a-zA-Z]*\n.*?```", "", raw, flags=re.DOTALL).strip()
    cleaned = [b.strip().lstrip("$").strip() for b in blocks if b.strip()]
    return {
        "command": cleaned[0] if cleaned else None,
        "alternatives": cleaned[1:],
        "explanation": rest,
    }


if __name__ == "__main__":
    demo = (
        "Here you go:\n\n```bash\nfind . -type f -size +100M\n```\n\n"
        "Finds big files.\n\n```\nfind . -type f -size +100M -print\n```"
    )
    print(parse_llm_answer(demo))
