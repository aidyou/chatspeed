## @src/views/Workflow.vue bug 修复
1. 页面切换后最后的审批状态还原后会出现无论是拒绝还是批准，都会无法关闭窗口，这会导致无法进行其他的任何操作，因为界面被遮住覆盖了，错误如下：
```log
15:42:51.930 [D] chatspeed_lib::workflow::react::gateway - src/workflow/react/gateway.rs:149 [Workflow Gateway] Injecting input for session 0pr4rcfbm0400: {"type":"approval","approved":false,"id":"call_1ec3bcaecd56490f8c8f5986"}
15:42:51.931 [W] chatspeed_lib::workflow::react::gateway - src/workflow/react/gateway.rs:161 [Workflow Gateway] Failed to inject input: No sender for session 0pr4rcfbm0400
```

2. 前端无论切换审批模式为「按智能体配置」还是「只能审批（只拦截写操作）」，都会弹窗类似 `git log`、`git diff` 等命令。这不符合我们的需求。智能模式应该只对有高度风险的操作审批，比如`rm`, `mv` 等。所以这里有2个调整：
 - 前端的「智能审批 (只拦截写操作)」这个 i18n 要改为「智能审批 (只拦截风险操作)」
 - 看下后端是不是没运用这个设置
下面是数据库中 agent.shell_prolicy的配置：
```json
[..., {"pattern":"^git log($| .*)","decision":"allow"},{"pattern":"^git diff($| .*)","decision":"allow"},{"pattern":"^git branch($| .*)","decision":"allow"},{"pattern":"^git remote($| .*)","decision":"allow"},{"pattern":"^git tag($| .*)","decision":"allow"},{"pattern":"^git rev-parse($| .*)","decision":"allow"},{"pattern":"^git config --list($| .*)","decision":"allow"}, ...]
```

另外，我看到 workflow 表只有allowed_paths、final_audit字段，没有 models、available_tools、shell_prolicy、auto_approve、approval_level 字段。为了方便以后拓展，索性增加一个 agent_config 用来存储这些智能体配置字段，去掉allowed_paths、final_audit（合并到配置字段）。由于 v5还没发布，所以直接添加到 @src-tauri/src/db/sql/migrations/v5.rs workflow 表即可。


## @src/views/Workflow.vue  页面优化
1. edit_file, write_file 前端审核和展示的应该是 js 的 diff 信息，目前显示的格式不对
3. 工具调用信息在前端展示或者审核弹窗展示的详细内容应该是便于阅读的格式，比如:
> command: ls -la pnpm-lock.yaml 2>&1 || echo "pnpm-lock.yaml not found"
3. 前端用户消息应该强制换行，现在在有些连续字符上不换行，导致界面有横向滚动条。
4. `/models` 模型配置后，我看到 ide 控台的日志后端应该是保存了，不过前端没生效（无论是接下来的工作流对话还是再次弹窗修改models）。

## @src/components/setting/Agent.vue 模型配置的时候去掉代码模型、写作模型、浏览模型，这些我觉得没必要。注意 rust 端也要删除对应模型相关的代码。

## 后端优化
1. 生成工作流标题，感觉应该是阻塞线程了，应该改为异步，不影响其他操作。
2. 优化步数耗尽时流程：步数耗尽后自动发送 ask_user 消息，询问用户：当前步数已耗尽但是任务尚未完成是否继续：是 -> 后端重置步数切换到 think 状态继续开始，继续执行任务；否 -> 后端结束任务。在审核模式为 Full 下，自动审核步数增加
3. shell运行的结果作为通知消息即时传递给前端展示——这个需要研究下可行性
4. 自动审批的命令，他的执行详情不会发送给前端，这样前端不到操作了啥，而且工具始终处于执行状态（不会更新完成）。
5. `Critical Error: 操作已被用户取消`用户取消操作不应该视为“关键错误”
6. 上下文压缩后，下一轮又触发压缩！
7. 当执行 shell 脚本时，全部审核应该加个规则而不是全匹配，比如如果执行 `cat /a/b/c`则全部同意的时候应该是 `cat *` 这个命令，对于 `pnpm dev --xxx` 这样的命令，应该同意 `pnpm dev *` 这个命令。另外，这些全部同意的应该更新到 workflow 表的 shell_prolicy 字段。
