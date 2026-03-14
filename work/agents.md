# 记忆支持
全局记忆文件：～/.chatspeed/memory.md
项目记忆文件：当前项目目录下的 .cs/memory.md 文件

> 我需要开发记忆支持功能，让智能体在运行过程中，根据用户的输入，捕获的项目偏好和全局偏好并写入记忆文件。需要注意捕获用户真正的输入信息，因为智能体运行过程当中的观察信息也会作为role:user存储数据表

# 智能体配置支持
AGENTS.md 文件支持，扫描目录：
    - ~/.chatspeed
    - 当前项目目录

系统提示词结构：核心提示词 -> 智能体提示词 -> 计划模式或工作模式提示词 -> 全局 AGENTS.md -> 本地 AGENTS.md -> 全局记忆文件 -> 项目记忆文件

示例结构：
You are a tool-driven autonomous AI Agent...

<AGENT_SPECIFIC_INSTRUCTIONS>
You are an expert interactive AI Agent that helps users with software engineering tasks. Use the instructions below and the tools available to you to assist the user...
</AGENT_SPECIFIC_INSTRUCTIONS>

<PHASE_INSTRUCTIONS>
Enter plan mode...
</PHASE_INSTRUCTIONS>

<GLOBAL_xxx>
全局 AGENTS.md

<SYSTEM_REMINDER>
This may or may not relate to the current project, you ...
</SYSTEM_REMINDER>
</GLOBAL_xxx>

<PROJECT_xxx>
本地 AGENTS.md
</PROJECT_xxx>

<GLOBAL_MEMORY>
全局记忆文件内容

<SYSTEM_REMINDER>
这些记忆可能与本项目无关也可能有关，请您仔细甄别，对于与项目相关的，请按用户的偏好行事。
</SYSTEM_REMINDER>
</GLOBAL_MEMORY>

<PROJECT_MEMORY>
项目记忆文件内容

<SYSTEM_REMINDER>
这些记忆通常与本项目相关，如果项目的记忆文件与全局记忆文件所记录的事项不同，请以本项目的记忆为准。
</SYSTEM_REMINDER>
</PROJECT_MEMORY>

---
注意，上述示例信息请用英文进行优化表达