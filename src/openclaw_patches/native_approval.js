// @wecode-patch weixin-native-approval-control
function parseWecodeApprovalControlText(text) {
    const match = String(text ?? "").trim().match(/^(?::?(yes|approve|no|deny))(?:\s+([\w-]+))?$/i);
    if (!match) return null;
    const action = match[1].toLowerCase();
    return {
        decision: action === "yes" || action === "approve" ? "approve" : "deny",
        approvalId: match[2],
    };
}
function resolveWecodeOpenClawStateDir() {
    const configured = process.env.OPENCLAW_STATE_DIR?.trim();
    const stateDir = configured && configured.length > 0 ? configured : "~/.wecode/openclaw-state";
    if (stateDir === "~") return process.env.HOME || process.env.USERPROFILE || ".";
    if (stateDir.startsWith("~/")) return path.join(process.env.HOME || process.env.USERPROFILE || ".", stateDir.slice(2));
    return path.resolve(stateDir);
}
function wecodeWeixinSentApprovalIds() {
    if (!(globalThis.__wecodeWeixinSentApprovalIds instanceof Set)) {
        globalThis.__wecodeWeixinSentApprovalIds = new Set();
    }
    return globalThis.__wecodeWeixinSentApprovalIds;
}
function wecodeNativeApprovalsDir() {
    return path.join(resolveWecodeOpenClawStateDir(), "approvals", "native");
}
function readWecodeNativeApproval(approvalId) {
    const approvalPath = path.join(wecodeNativeApprovalsDir(), `${approvalId}.json`);
    try {
        const record = JSON.parse(fs.readFileSync(approvalPath, "utf8"));
        const expiresAt = Number(record?.expires_at_millis ?? 0);
        if (expiresAt > 0 && expiresAt < Date.now()) return null;
        return { approvalId, approvalPath, record };
    }
    catch (err) {
        if (err?.code !== "ENOENT") {
            logger.warn(`wecode approval control read failed approvalId=${approvalId} err=${String(err)}`);
        }
        return null;
    }
}
function listWecodePendingNativeApprovalIds() {
    const dir = wecodeNativeApprovalsDir();
    try {
        return fs.readdirSync(dir, { withFileTypes: true })
            .filter((entry) => entry.isFile() && entry.name.endsWith(".json") && !entry.name.endsWith(".decision.json"))
            .map((entry) => entry.name.slice(0, -".json".length))
            .filter((approvalId) => Boolean(readWecodeNativeApproval(approvalId)))
            .sort();
    }
    catch (err) {
        if (err?.code !== "ENOENT") {
            logger.warn(`wecode approval control list failed dir=${dir} err=${String(err)}`);
        }
        return [];
    }
}
function readWecodeNativeApprovalPromptRecord(approvalId) {
    const approval = readWecodeNativeApproval(approvalId);
    if (!approval) return null;
    if (fs.existsSync(path.join(wecodeNativeApprovalsDir(), `${approvalId}.decision.json`))) return null;
    const prompt = String(approval.record?.prompt ?? "").trim();
    if (!prompt) return null;
    return { approvalId, prompt };
}
function listWecodePendingNativeApprovalPromptRecords() {
    return listWecodePendingNativeApprovalIds()
        .map((approvalId) => readWecodeNativeApprovalPromptRecord(approvalId))
        .filter(Boolean);
}
function startWecodeWeixinNativeApprovalWatcher(params) {
    const sentApprovalIds = wecodeWeixinSentApprovalIds();
    let scanning = false;
    const scan = async () => {
        if (scanning) return;
        scanning = true;
        try {
            for (const record of listWecodePendingNativeApprovalPromptRecords()) {
                const approvalId = record.approvalId;
                if (sentApprovalIds.has(approvalId)) continue;
                sentApprovalIds.add(approvalId);
                logger.info(`wecode approval watcher detected approvalId=${approvalId} textLen=${record.prompt.length}`);
                try {
                    await sendMessageWeixin({
                        to: params.to,
                        text: record.prompt,
                        opts: {
                            baseUrl: params.baseUrl,
                            token: params.token,
                            contextToken: params.contextToken,
                            runId: params.runId,
                        },
                    });
                    logger.info(`wecode approval watcher sent approvalId=${approvalId}`);
                }
                catch (err) {
                    sentApprovalIds.delete(approvalId);
                    logger.error(`wecode approval watcher send failed approvalId=${approvalId} err=${String(err)}`);
                }
            }
        }
        catch (err) {
            logger.warn(`wecode approval watcher scan failed err=${String(err)}`);
        }
        finally {
            scanning = false;
        }
    };
    void scan();
    const timer = setInterval(() => { void scan(); }, 1000);
    if (typeof timer.unref === "function") timer.unref();
    return timer;
}
function resolveWecodeNativeApprovalId(requestedApprovalId) {
    const approvalId = requestedApprovalId?.trim();
    if (approvalId) {
        return readWecodeNativeApproval(approvalId)
            ? { status: "ok", approvalId }
            : { status: "not_found", approvalId };
    }
    const ids = listWecodePendingNativeApprovalIds();
    if (ids.length === 1) return { status: "ok", approvalId: ids[0] };
    if (ids.length === 0) return { status: "none" };
    return { status: "multiple", ids };
}
async function sendWecodeApprovalControlMessage(params, text) {
    try {
        await sendMessageWeixin({
            to: params.to,
            text,
            opts: {
                baseUrl: params.baseUrl,
                token: params.token,
                contextToken: params.contextToken,
                runId: params.runId,
            },
        });
    }
    catch (err) {
        logger.error(`wecode approval control ack failed err=${String(err)}`);
    }
}
async function handleWecodeNativeApprovalControl(params) {
    const approval = parseWecodeApprovalControlText(params.text);
    if (!approval) return false;
    const resolved = resolveWecodeNativeApprovalId(approval.approvalId);
    if (resolved.status === "none") return false;
    if (resolved.status === "not_found") {
        await sendWecodeApprovalControlMessage(params, `Approval ${resolved.approvalId} was not found.`);
        return true;
    }
    if (resolved.status === "multiple") {
        await sendWecodeApprovalControlMessage(params, `Multiple pending approvals: ${resolved.ids.join(", ")}. Reply :yes <id> or :no <id>.`);
        return true;
    }
    const approvalId = resolved.approvalId;
    const decisionPath = path.join(wecodeNativeApprovalsDir(), `${approvalId}.decision.json`);
    try {
        fs.mkdirSync(wecodeNativeApprovalsDir(), { recursive: true });
        fs.writeFileSync(decisionPath, JSON.stringify({
            approval_id: approvalId,
            decision: approval.decision,
            decided_at_millis: Date.now(),
        }, null, 2));
        logger.info(`wecode approval control wrote decision approvalId=${approvalId} decision=${approval.decision}`);
    }
    catch (err) {
        logger.error(`wecode approval control write failed approvalId=${approvalId} err=${String(err)}`);
        await sendWecodeApprovalControlMessage(params, `Failed to update Codex approval ${approvalId}: ${String(err)}`);
        return true;
    }
    return true;
}
// @wecode-patch-end weixin-native-approval-control

// @wecode-patch weixin-sent-approval-ids
function wecodeWeixinSentApprovalIds() {
    if (!(globalThis.__wecodeWeixinSentApprovalIds instanceof Set)) {
        globalThis.__wecodeWeixinSentApprovalIds = new Set();
    }
    return globalThis.__wecodeWeixinSentApprovalIds;
}
// @wecode-patch-end weixin-sent-approval-ids

// @wecode-patch weixin-native-approval-watcher-helpers
function readWecodeNativeApprovalPromptRecord(approvalId) {
    const approval = readWecodeNativeApproval(approvalId);
    if (!approval) return null;
    if (fs.existsSync(path.join(wecodeNativeApprovalsDir(), `${approvalId}.decision.json`))) return null;
    const prompt = String(approval.record?.prompt ?? "").trim();
    if (!prompt) return null;
    return { approvalId, prompt };
}
function listWecodePendingNativeApprovalPromptRecords() {
    return listWecodePendingNativeApprovalIds()
        .map((approvalId) => readWecodeNativeApprovalPromptRecord(approvalId))
        .filter(Boolean);
}
function startWecodeWeixinNativeApprovalWatcher(params) {
    const sentApprovalIds = wecodeWeixinSentApprovalIds();
    let scanning = false;
    const scan = async () => {
        if (scanning) return;
        scanning = true;
        try {
            for (const record of listWecodePendingNativeApprovalPromptRecords()) {
                const approvalId = record.approvalId;
                if (sentApprovalIds.has(approvalId)) continue;
                sentApprovalIds.add(approvalId);
                logger.info(`wecode approval watcher detected approvalId=${approvalId} textLen=${record.prompt.length}`);
                try {
                    await sendMessageWeixin({
                        to: params.to,
                        text: record.prompt,
                        opts: {
                            baseUrl: params.baseUrl,
                            token: params.token,
                            contextToken: params.contextToken,
                            runId: params.runId,
                        },
                    });
                    logger.info(`wecode approval watcher sent approvalId=${approvalId}`);
                }
                catch (err) {
                    sentApprovalIds.delete(approvalId);
                    logger.error(`wecode approval watcher send failed approvalId=${approvalId} err=${String(err)}`);
                }
            }
        }
        catch (err) {
            logger.warn(`wecode approval watcher scan failed err=${String(err)}`);
        }
        finally {
            scanning = false;
        }
    };
    void scan();
    const timer = setInterval(() => { void scan(); }, 1000);
    if (typeof timer.unref === "function") timer.unref();
    return timer;
}
// @wecode-patch-end weixin-native-approval-watcher-helpers

// @wecode-patch feishu-native-approval-control
function parseWecodeApprovalControlText(text) {
	const match = String(text ?? "").trim().match(/^(?::?(yes|approve|no|deny))(?:\s+([\w-]+))?$/i);
	if (!match) return null;
	const action = match[1].toLowerCase();
	return {
		decision: action === "yes" || action === "approve" ? "approve" : "deny",
		approvalId: match[2],
	};
}
function resolveWecodeOpenClawStateDir() {
	const configured = process.env.OPENCLAW_STATE_DIR?.trim();
	const stateDir = configured && configured.length > 0 ? configured : "~/.wecode/openclaw-state";
	if (stateDir === "~") return process.env.HOME || process.env.USERPROFILE || ".";
	if (stateDir.startsWith("~/")) return path.join(process.env.HOME || process.env.USERPROFILE || ".", stateDir.slice(2));
	return path.resolve(stateDir);
}
function wecodeNativeApprovalsDir() {
	return path.join(resolveWecodeOpenClawStateDir(), "approvals", "native");
}
function readWecodeNativeApproval(approvalId, log) {
	const approvalPath = path.join(wecodeNativeApprovalsDir(), `${approvalId}.json`);
	try {
		const record = JSON.parse(fs.readFileSync(approvalPath, "utf8"));
		const expiresAt = Number(record?.expires_at_millis ?? 0);
		if (expiresAt > 0 && expiresAt < Date.now()) return null;
		return { approvalId, approvalPath, record };
	}
	catch (err) {
		if (err?.code !== "ENOENT") {
			log(`wecode approval control read failed approvalId=${approvalId} err=${String(err)}`);
		}
		return null;
	}
}
function listWecodePendingNativeApprovalIds(log) {
	const dir = wecodeNativeApprovalsDir();
	try {
		return fs.readdirSync(dir, { withFileTypes: true })
			.filter((entry) => entry.isFile() && entry.name.endsWith(".json") && !entry.name.endsWith(".decision.json"))
			.map((entry) => entry.name.slice(0, -".json".length))
			.filter((approvalId) => Boolean(readWecodeNativeApproval(approvalId, log)))
			.sort();
	}
	catch (err) {
		if (err?.code !== "ENOENT") {
			log(`wecode approval control list failed dir=${dir} err=${String(err)}`);
		}
		return [];
	}
}
function resolveWecodeNativeApprovalId(requestedApprovalId, log) {
	const approvalId = requestedApprovalId?.trim();
	if (approvalId) {
		return readWecodeNativeApproval(approvalId, log)
			? { status: "ok", approvalId }
			: { status: "not_found", approvalId };
	}
	const ids = listWecodePendingNativeApprovalIds(log);
	if (ids.length === 1) return { status: "ok", approvalId: ids[0] };
	if (ids.length === 0) return { status: "none" };
	return { status: "multiple", ids };
}
async function sendWecodeApprovalControlMessage(params, text) {
	try {
		await sendMessageFeishu({
			cfg: params.cfg,
			to: `chat:${params.chatId}`,
			text,
			accountId: params.accountId,
		});
	}
	catch (err) {
		params.error(`wecode approval control ack failed err=${String(err)}`);
	}
}
async function handleWecodeNativeApprovalControl(params) {
	const log = typeof params.log === "function" ? params.log : console.log;
	const error = typeof params.error === "function" ? params.error : console.error;
	const approval = parseWecodeApprovalControlText(params.text);
	if (!approval) return false;
	const resolved = resolveWecodeNativeApprovalId(approval.approvalId, log);
	if (resolved.status === "none") return false;
	if (resolved.status === "not_found") {
		await sendWecodeApprovalControlMessage({ ...params, error }, `Approval ${resolved.approvalId} was not found.`);
		return true;
	}
	if (resolved.status === "multiple") {
		await sendWecodeApprovalControlMessage({ ...params, error }, `Multiple pending approvals: ${resolved.ids.join(", ")}. Reply :yes <id> or :no <id>.`);
		return true;
	}
	const approvalId = resolved.approvalId;
	const decisionPath = path.join(wecodeNativeApprovalsDir(), `${approvalId}.decision.json`);
	try {
		fs.mkdirSync(wecodeNativeApprovalsDir(), { recursive: true });
		fs.writeFileSync(decisionPath, JSON.stringify({
			approval_id: approvalId,
			decision: approval.decision,
			decided_at_millis: Date.now(),
		}, null, 2));
		log(`wecode approval control wrote decision approvalId=${approvalId} decision=${approval.decision}`);
	}
	catch (err) {
		error(`wecode approval control write failed approvalId=${approvalId} err=${String(err)}`);
		await sendWecodeApprovalControlMessage({ ...params, error }, `Failed to update Codex approval ${approvalId}: ${String(err)}`);
		return true;
	}
	return true;
}
// @wecode-patch-end feishu-native-approval-control

// @wecode-patch stop-control-helpers
function parseWecodeStopControlText(text) {
    return String(text ?? "").trim().toLowerCase() === ":stop";
}
function wecodeRunLocksDir() {
    return path.join(resolveWecodeOpenClawStateDir(), "locks");
}
function listWecodeRunLockPaths(log = () => {}) {
    const dir = wecodeRunLocksDir();
    try {
        return fs.readdirSync(dir, { withFileTypes: true })
            .filter((entry) => entry.isFile() && entry.name.startsWith("codex-run-") && entry.name.endsWith(".lock"))
            .map((entry) => path.join(dir, entry.name))
            .sort();
    }
    catch (err) {
        if (err?.code !== "ENOENT") log(`wecode stop lock list failed dir=${dir} err=${String(err)}`);
        return [];
    }
}
function readWecodeRunLockPid(lockPath, log = () => {}) {
    try {
        const content = fs.readFileSync(lockPath, "utf8");
        const line = content.split(/\r?\n/).find((item) => item.startsWith("pid="));
        const pid = Number(line?.slice("pid=".length).trim());
        return Number.isInteger(pid) && pid > 0 ? pid : null;
    }
    catch (err) {
        if (err?.code !== "ENOENT") log(`wecode stop lock read failed path=${lockPath} err=${String(err)}`);
        return null;
    }
}
function wecodeProcessIsAlive(pid) {
    if (!Number.isInteger(pid) || pid <= 0 || pid === process.pid) return false;
    try {
        process.kill(pid, 0);
        return true;
    }
    catch (err) {
        return err?.code === "EPERM";
    }
}
async function wecodeChildPids(pid, log = () => {}) {
    try {
        const { execFileSync } = await import("node:child_process");
        if (process.platform === "win32") {
            const output = execFileSync("wmic", ["process", "where", `ParentProcessId=${pid}`, "get", "ProcessId", "/FORMAT:LIST"], {
                encoding: "utf8",
                stdio: ["ignore", "pipe", "ignore"],
            });
            return output.split(/\r?\n/)
                .filter((line) => line.startsWith("ProcessId="))
                .map((line) => Number(line.slice("ProcessId=".length).trim()))
                .filter((child) => Number.isInteger(child) && child > 0 && child !== process.pid);
        }
        const output = execFileSync("/usr/bin/pgrep", ["-P", String(pid)], {
            encoding: "utf8",
            stdio: ["ignore", "pipe", "ignore"],
        });
        return output.split(/\r?\n/)
            .map((line) => Number(line.trim()))
            .filter((child) => Number.isInteger(child) && child > 0 && child !== process.pid);
    }
    catch (err) {
        if (err?.status !== 1) log(`wecode stop child pid lookup failed pid=${pid} err=${String(err)}`);
        return [];
    }
}
async function wecodeDescendantPids(pid, log = () => {}, seen = new Set()) {
    if (seen.has(pid)) return [];
    seen.add(pid);
    const result = [];
    for (const child of await wecodeChildPids(pid, log)) {
        result.push(...await wecodeDescendantPids(child, log, seen), child);
    }
    return result;
}
function signalWecodeProcess(pid, signal, log = () => {}) {
    if (!Number.isInteger(pid) || pid <= 0 || pid === process.pid) return;
    try {
        process.kill(pid, signal);
    }
    catch (err) {
        if (err?.code !== "ESRCH") log(`wecode stop signal failed pid=${pid} signal=${signal} err=${String(err)}`);
    }
}
async function stopWecodeProcessTree(pid, log = () => {}) {
    const descendants = await wecodeDescendantPids(pid, log);
    for (const child of descendants.slice().reverse()) signalWecodeProcess(child, "SIGTERM", log);
    signalWecodeProcess(pid, "SIGTERM", log);
    await new Promise((resolve) => setTimeout(resolve, 500));
    if (wecodeProcessIsAlive(pid)) {
        for (const child of descendants.slice().reverse()) signalWecodeProcess(child, "SIGKILL", log);
        signalWecodeProcess(pid, "SIGKILL", log);
    }
}
function removeWecodeFileIfExists(filePath, log = () => {}) {
    try {
        fs.unlinkSync(filePath);
    }
    catch (err) {
        if (err?.code !== "ENOENT") log(`wecode stop file cleanup failed path=${filePath} err=${String(err)}`);
    }
}
function clearWecodeJsonFiles(dir, log = () => {}) {
    try {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            if (entry.isFile() && entry.name.endsWith(".json")) {
                removeWecodeFileIfExists(path.join(dir, entry.name), log);
            }
        }
    }
    catch (err) {
        if (err?.code !== "ENOENT") log(`wecode stop approval cleanup failed dir=${dir} err=${String(err)}`);
    }
}
function clearWecodePendingApprovals(log = () => {}) {
    const approvalsDir = path.join(resolveWecodeOpenClawStateDir(), "approvals");
    clearWecodeJsonFiles(approvalsDir, log);
    clearWecodeJsonFiles(path.join(approvalsDir, "native"), log);
}
async function stopWecodeCodexRuns(log = () => {}) {
    let stopped = false;
    for (const lockPath of listWecodeRunLockPaths(log)) {
        const pid = readWecodeRunLockPid(lockPath, log);
        if (pid && wecodeProcessIsAlive(pid)) {
            await stopWecodeProcessTree(pid, log);
            stopped = true;
        }
        removeWecodeFileIfExists(lockPath, log);
    }
    return stopped;
}
// @wecode-patch-end stop-control-helpers

// @wecode-patch approval-control-text-helper
function isWecodeApprovalControlText(text) {
	const normalized = String(text ?? "").trim().toLowerCase();
	if (normalized === ":stop") return true;
	return /^(?::?(?:yes|no|approve|deny))(?:\s+[\w-]+)?$/.test(normalized);
}
// @wecode-patch-end approval-control-text-helper

// @wecode-patch weixin-approval-partial-reply-options
...(replyProgressSender?.replyOptions ?? {}),
                    ...(() => {
                        const wecodeBaseReplyOptions = {
                            ...replyOptions,
                            ...(replyProgressSender?.replyOptions ?? {}),
                        };
                        const wecodeSentApprovalIds = wecodeWeixinSentApprovalIds();
                        const sendWecodeApprovalPromptFromReplyPayload = async (payload) => {
                            const text = String(payload?.text ?? "");
                            const approvalId = text.match(/审批 ID\**:\s*`?(appr-[\w-]+)`?/)?.[1]
                                ?? text.match(/Approval:\s*(appr-[\w-]+)/)?.[1];
                            if (!approvalId || wecodeSentApprovalIds.has(approvalId)) return;
                            wecodeSentApprovalIds.add(approvalId);
                            logger.info(`wecode approval prompt detected approvalId=${approvalId} textLen=${text.length}`);
                            try {
                                await sendMessageWeixin({
                                    to: ctx.To,
                                    text,
                                    opts: { baseUrl: deps.baseUrl, token: deps.token, contextToken, runId },
                                });
                                logger.info(`wecode approval prompt sent approvalId=${approvalId}`);
                            }
                            catch (err) {
                                wecodeSentApprovalIds.delete(approvalId);
                                logger.error(`wecode approval prompt send failed approvalId=${approvalId} err=${String(err)}`);
                            }
                        };
                        const callWecodePreviousReplyOption = async (name, payload, context) => {
                            const previous = wecodeBaseReplyOptions?.[name];
                            if (typeof previous !== "function") return;
                            try {
                                await previous(payload, context);
                            }
                            catch (err) {
                                logger.warn(`wecode approval prompt previous ${name} failed err=${String(err)}`);
                            }
                        };
                        return {
                            onPartialReply: async (payload) => {
                                await callWecodePreviousReplyOption("onPartialReply", payload);
                                await sendWecodeApprovalPromptFromReplyPayload(payload);
                            },
                            onBlockReplyQueued: async (payload, context) => {
                                await callWecodePreviousReplyOption("onBlockReplyQueued", payload, context);
                                await sendWecodeApprovalPromptFromReplyPayload(payload);
                            },
                        };
                    })(),
                    disableBlockStreaming: false,
// @wecode-patch-end weixin-approval-partial-reply-options

// @wecode-patch weixin-monitor-lane-enqueue
                const laneKey = getWeixinLaneKey({ accountId, msg: full });
                void wecodeLaneScheduler.enqueue(laneKey, async () => {
                    const fromUserId = full.from_user_id ?? "";
                    const cachedConfig = await configManager.getForUser(fromUserId, full.context_token);
                    await processOneMessage(full, {
                        accountId,
                        config,
                        channelRuntime,
                        baseUrl,
                        cdnBaseUrl,
                        token,
                        typingTicket: cachedConfig.typingTicket,
                        log: opts.runtime?.log ?? (() => { }),
                        errLog,
                    });
                }).catch((err) => {
                    errLog(`weixin processOneMessage error lane=${laneKey}: ${String(err)}`);
                    aLog.error(`processOneMessage lane=${laneKey} error: ${String(err)}, stack=${err?.stack ?? ""}`);
                });
// @wecode-patch-end weixin-monitor-lane-enqueue

// @wecode-patch feishu-approval-partial-reply-options
			...(() => {
				const wecodeFeishuBaseReplyOptions = {
					...replyOptions,
				};
				const wecodeFeishuSentApprovalIds = new Set();
				const sendWecodeApprovalPromptFromReplyPayload = async (payload) => {
					const text = String(payload?.text ?? "");
					const approvalId = text.match(/审批 ID\**:\s*`?(appr-[\w-]+)`?/)?.[1]
						?? text.match(/Approval:\s*(appr-[\w-]+)/)?.[1];
					if (!approvalId || wecodeFeishuSentApprovalIds.has(approvalId)) return;
					wecodeFeishuSentApprovalIds.add(approvalId);
					params.runtime.log?.(`wecode feishu approval prompt detected approvalId=${approvalId} textLen=${text.length}`);
					try {
						await sendMessageFeishu({
							cfg,
							to: chatId,
							text,
							replyToMessageId: sendReplyToMessageId,
							replyInThread: effectiveReplyInThread,
							allowTopLevelReplyFallback,
							accountId,
						});
						params.runtime.log?.(`wecode feishu approval prompt sent approvalId=${approvalId}`);
					}
					catch (err) {
						wecodeFeishuSentApprovalIds.delete(approvalId);
						params.runtime.error?.(`wecode feishu approval prompt send failed approvalId=${approvalId} err=${String(err)}`);
					}
				};
				const callWecodeFeishuPreviousReplyOption = async (name, payload, context) => {
					const previous = wecodeFeishuBaseReplyOptions?.[name];
					if (typeof previous !== "function") return;
					try {
						await previous(payload, context);
					}
					catch (err) {
						params.runtime.log?.(`wecode feishu approval prompt previous ${name} failed err=${String(err)}`);
					}
				};
				return {
					onPartialReply: async (payload) => {
						await callWecodeFeishuPreviousReplyOption("onPartialReply", payload);
						if (streamingEnabled) {
							if (payload.text) {
								const cleaned = stripReasoningTagsFromText(payload.text, {
									mode: "strict",
									trim: "both"
								});
								if (cleaned) {
									queueStreamingUpdate(cleaned, {
										dedupeWithLastPartial: true,
										mode: "snapshot"
									});
								}
							}
						}
						await sendWecodeApprovalPromptFromReplyPayload(payload);
					},
					onBlockReplyQueued: async (payload, context) => {
						await callWecodeFeishuPreviousReplyOption("onBlockReplyQueued", payload, context);
						await sendWecodeApprovalPromptFromReplyPayload(payload);
					},
				};
			})(),
// @wecode-patch-end feishu-approval-partial-reply-options
