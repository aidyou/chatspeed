---
name: skill-creator
description: Create new skills, modify and improve existing skills. Use when users want to create a skill from scratch, edit an existing skill, or improve a skill's description for better triggering accuracy.
---
# Skill Creator

A skill for creating new skills and iteratively improving them.

## Quick Start

1. **Define skill requirements** — Discuss with the user what the skill should do
2. **Write SKILL.md** — Follow the template below
3. **Install the skill** — Copy directly to `~/.chatspeed/skills/`
4. **Test and iterate** — Test and optimize in conversations

---

## Creating a Skill

### 1. Understand User Requirements

Start by understanding the user's intent:

1. What should this skill enable the AI to do?
2. When to trigger? (When does the user invoke this skill)
3. What is the expected output format?
4. Should test cases be created?

Ask proactively about edge cases, input/output formats, example files, and success criteria.

### 2. Write SKILL.md

Fill in according to requirements:

```yaml
---
name: skill-name
description: Skill description (trigger condition + functionality)
---
# Skill Name

## Function Description

(Detailed explanation)
```

**name**: Skill identifier, English + hyphens

**description**: Trigger condition + functionality. This is the main triggering mechanism — explain what the skill does and when to use it. All "when to trigger" information goes here, not in the body.

> **Tip**: Make descriptions slightly "proactive". For example, don't just write "how to build a dashboard", write "how to build a dashboard. Use this skill when the user mentions dashboards, data visualization, internal metrics, or wants to display any type of data."

### 3. Skill Directory Structure

```
skill-name/
├── SKILL.md (required)
│   ├── YAML frontmatter (name, description required)
│   └── Markdown explanation
└── Resource files (optional)
    ├── scripts/    - Helper scripts
    ├── references/ - Reference documents
    └── assets/     - Asset files
```

### 4. Install the Skill

Copy the skill folder directly to `~/.chatspeed/skills/`:

```bash
cp -r skill-name/ ~/.chatspeed/skills/
```

ChatSpeed will automatically load skills from this directory.

---

## Testing and Optimization

### Testing the Skill

1. Tell the user you've created the skill
2. Give the user 2-3 test queries
3. After user confirmation, use this skill in the current conversation
4. Evaluate results and discuss improvements with the user

### Optimizing Descriptions

Skill description optimization can be done directly in the conversation:

```
Help me optimize this skill's description:
Skill name: xxx
Current description: xxx

Failed test cases:
- query: "xxx" (should trigger but didn't)
- query: "xxx" (triggered but shouldn't have)

Successful test cases:
- query: "xxx" (triggered correctly)

Please provide a better description.
```

Optimization tips:
- Keep descriptions between 100-200 words
- Focus on user intent, not implementation details
- Make it distinctive, able to differentiate from other skills
- If multiple attempts fail, try a different way of expressing it

---

## Skill Writing Guide

### Writing Style

- Use imperative mood
- Explain why it's important, don't just list MUSTs
- Example:

```markdown
## Output Format

Always use this template:
# [Title]
## Summary
## Key Findings
## Recommendations
```

```markdown
## Examples

**Example 1:**
Input: Add user authentication
Output: feat(auth): implement JWT-based authentication
```

### Keep It Concise

- Keep SKILL.md under 500 lines
- Add table of contents for large reference files (>300 lines)
- Organize complex skills by domain:

```
cloud-deploy/
├── SKILL.md
└── references/
    ├── aws.md
    ├── gcp.md
    └── azure.md
```

---

## Communicating with Users

This skill may be used by users with varying technical proficiency. Note:
- "evaluation", "benchmark" are fine to use
- "JSON", "assertion" — check if the user understands
- When in doubt, provide a simple explanation

---

## Saving Test Cases (Optional)

If test cases need to be saved, create `evals/evals.json`:

```json
{
  "skill_name": "example-skill",
  "evals": [
    {
      "id": 1,
      "prompt": "User's task description",
      "expected_output": "Expected result description",
      "files": []
    }
  ]
}
```

---

Feel free to ask the user questions anytime. Creating skills is an iterative process — it doesn't need to be perfect from the start.