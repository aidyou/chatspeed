---
name: skill-creator
description: Create new skills, modify and improve existing skills. Use when users want to create a skill from scratch, edit an existing skill, or improve a skill's description for better triggering accuracy.
---
# Skill Creator

A skill for creating new skills and iteratively improving them.

## Quick Start

1. **确定技能需求** — 和用户聊清楚要做什么
2. **编写 SKILL.md** — 按照下面的模板
3. **安装技能** — 直接复制到 `~/.chatspeed/skills/`
4. **测试迭代** — 在对话中测试和优化

---

## 创建技能流程

### 1. 理解用户需求

Start by understanding the user's intent:

1. 这个技能让 AI 能做什么？
2. 什么时候触发？（用户说什么时调用这个技能）
3. 期望的输出格式是什么？
4. 需要设置测试用例吗？

主动问边缘情况、输入输出格式、示例文件、成功标准。

### 2. 编写 SKILL.md

根据需求填写：

```yaml
---
name: skill-name
description: 技能描述（触发条件 + 功能）
---
# 技能名称

## 功能说明

（详细说明）
```

**name**: 技能标识符，英文+连字符

**description**: 触发条件 + 功能。这是主要的触发机制——既要说明技能做什么，也要说明在什么情况下使用。所有"何时触发"的信息写在这里，不是正文。

> **提示**：描述要稍微"主动"一点。比如不要只写"如何构建仪表板"，而要写"如何构建仪表板。当用户提到仪表板、数据可视化、内部指标，或想展示任何类型的数据时，使用此技能。"

### 3. 技能目录结构

```
skill-name/
├── SKILL.md (必需)
│   ├── YAML frontmatter (name, description 必需)
│   └── Markdown 说明
└── 资源文件 (可选)
    ├── scripts/    - 辅助脚本
    ├── references/ - 参考文档
    └── assets/     - 资源文件
```

### 4. 安装技能

直接把技能文件夹复制到 `~/.chatspeed/skills/`:

```bash
cp -r skill-name/ ~/.chatspeed/skills/
```

ChatSpeed 会自动加载该目录下的技能。

---

## 测试和优化

### 测试技能

1. 告诉用户你已经创建了技能
2. 给用户 2-3 个测试 query
3. 用户确认后，在当前对话中使用这个技能
4. 评估结果，和用户讨论改进点

### 优化描述

技能描述优化可以直接在对话中进行：

```
帮我优化一下这个技能的描述：
技能名称: xxx
当前描述: xxx

失败的测试案例:
- query: "xxx" (应该触发但没触发)
- query: "xxx" (触发了但不应该)

成功的测试案例:
- query: "xxx" (正确触发)

请给出一个更好的描述。
```

优化要点：
- 描述控制在 100-200 字
- 关注用户意图，而不是实现细节
- 具有辨识度，能和其他技能区分
- 如果多次尝试都失败，尝试换一种表达方式

---

## 技能写作指南

### 编写风格

- 使用祈使句
- 解释为什么重要，而不是堆砌 MUST
- 例子：

```markdown
## 输出格式

始终使用这个模板：
# [标题]
## 总结
## 关键发现
## 建议
```

```markdown
## 示例

**示例 1:**
输入: 添加用户认证
输出: feat(auth): implement JWT-based authentication
```

### 保持简洁

- SKILL.md 控制在 500 行以内
- 大型参考文件（>300 行）加目录
- 复杂技能按领域组织：

```
cloud-deploy/
├── SKILL.md
└── references/
    ├── aws.md
    ├── gcp.md
    └── azure.md
```

---

## 与用户沟通

这个技能可能被不同技术水平的用户使用。注意：
- "evaluation"、"benchmark" 可以用
- "JSON"、"assertion" 需要看用户是否懂
- 不确定时可以简单解释一下

---

## 保存测试用例（可选）

如果需要保存测试用例，创建 `evals/evals.json`:

```json
{
  "skill_name": "example-skill",
  "evals": [
    {
      "id": 1,
      "prompt": "用户的任务描述",
      "expected_output": "期望结果描述",
      "files": []
    }
  ]
}
```

---

有问题随时问用户。创建技能是一个迭代过程，不需要一步到位。