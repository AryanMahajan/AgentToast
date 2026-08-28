// The Connectors tab: wiring each agent's hooks up, and the one switch that
// decides whether an Antigravity toast can approve anything.

import { useState } from 'react';
import {
    addProject,
    agyApprovalStatus,
    agyStatus,
    chooseFolder,
    claudeStatus,
    connectAgy,
    connectClaude,
    disableAgyApproval,
    disconnectAgy,
    disconnectClaude,
    enableAgyApproval,
    removeProject,
    usePolled,
} from './api.ts';
import type { AgyApprovalStatus, HookStatus } from './types.ts';
import { Button, Dot, Hint, Panel, type Tone } from './ui.tsx';

const POLL_MS = 4000;

function folderName(path: string | null): string {
    if (!path) return 'This project';
    const parts = path.replace(/\\/g, '/').split('/').filter(Boolean);
    return parts[parts.length - 1] ?? path;
}

/** Connected and pointing at this build is green; connected to a bridge that no
 *  longer exists is a warning, because it silently does nothing. */
function statusTone(status: HookStatus): Tone {
    if (status.stale) return 'warn';
    return status.connected ? 'ok' : 'fg3';
}

/** One row of a connector panel: a scope, where it is written, and a button. */
function HookRow({
    status,
    globalLabel,
    onConnect,
    onDisconnect,
    onRemove,
    onDone,
    onError,
}: {
    status: HookStatus;
    globalLabel: string;
    onConnect: (scope: string, project: string | null) => Promise<unknown>;
    onDisconnect: (scope: string, project: string | null) => Promise<unknown>;
    onRemove?: (project: string) => Promise<unknown>;
    onDone: () => void;
    onError: (message: string) => void;
}) {
    const [busy, setBusy] = useState(false);
    const scope = status.project ? 'project' : 'global';
    const connected = status.connected && !status.stale;

    const detail = status.stale
        ? `points at a different build: ${status.bridge}`
        : status.connected
          ? status.path
          : `not connected — ${status.path}`;

    async function toggle() {
        setBusy(true);
        try {
            await (connected ? onDisconnect : onConnect)(scope, status.project);
        } catch (e) {
            console.error('Setup failed:', e);
            onError(String(e));
        }
        setBusy(false);
        onDone();
    }

    return (
        <div className="flex min-w-0 items-center gap-2.5">
            <Dot tone={statusTone(status)} className="h-2 w-2" />
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="font-ui text-xs leading-tight font-semibold text-fg">
                    {status.project ? folderName(status.project) : globalLabel}
                </div>
                <div
                    className={`path-ellipsis font-mono text-[10.5px] leading-tight ${status.stale ? 'text-warn' : 'text-fg3'}`}
                    title={detail}
                >
                    {detail}
                </div>
            </div>
            <Button
                variant={connected ? 'outline' : 'primary'}
                disabled={busy}
                onClick={() => void toggle()}
            >
                {status.stale ? 'Repair' : status.connected ? 'Disconnect' : 'Connect'}
            </Button>
            {status.project && onRemove ? (
                <Button
                    variant="ghost"
                    className="px-2 py-1.5 text-sm leading-none"
                    title="Remove this project from the list (its hooks are left alone)"
                    onClick={async () => {
                        try {
                            await onRemove(status.project as string);
                        } catch (e) {
                            console.error('Could not remove project:', e);
                        }
                        onDone();
                    }}
                >
                    ×
                </Button>
            ) : null}
        </div>
    );
}

function ClaudePanel({ active }: { active: boolean }) {
    const { data, error, refresh } = usePolled(claudeStatus, POLL_MS, active);
    const [failure, setFailure] = useState<string | null>(null);

    const statuses = data ?? [];
    const connected = statuses.some((s) => s.connected && !s.stale);

    const note = failure
        ? failure
        : error
          ? 'Could not read Claude Code settings'
          : data && !connected
            ? 'Not connected — Claude Code will not send anything yet'
            : undefined;

    async function pickProject() {
        try {
            const picked = await chooseFolder();
            if (!picked) return;
            // Recorded by the daemon, so the row is still here next time the
            // dashboard opens — whether or not it gets connected now.
            await addProject(String(picked));
            setFailure(null);
            await refresh();
        } catch (e) {
            console.error('Folder picker failed:', e);
            setFailure('Could not open the folder picker');
        }
    }

    return (
        <div className="flex flex-col gap-2">
            <Panel
                title="Claude Code"
                note={note}
                noteTone={failure || error ? 'warn' : 'fg3'}
                footer={
                    <Button variant="ghost" className="self-start" onClick={() => void pickProject()}>
                        Add a project…
                    </Button>
                }
            >
                {statuses.map((status) => (
                    <HookRow
                        key={status.project ?? 'global'}
                        status={status}
                        globalLabel="Every project"
                        onConnect={connectClaude}
                        onDisconnect={disconnectClaude}
                        onRemove={removeProject}
                        onDone={() => void refresh()}
                        onError={setFailure}
                    />
                ))}
            </Panel>
            <Hint>
                Connecting adds AgentToast to Claude Code’s settings and backs the file up first.
                Anything already configured there is left alone.
            </Hint>
        </div>
    );
}

/**
 * Which calls raise a toast.
 *
 * This is here because Antigravity gates commands and file edits through two
 * different mechanisms. Commands go through the permission lists in every
 * execution mode, so a toast for one is always right. File edits go through the
 * *execution mode*: `default` pauses for a diff review, and `accept-edits`
 * deliberately does not pause at all — where a toast reintroduces exactly the
 * interruption the mode was chosen to remove.
 *
 * A hook cannot tell which mode it is in; the payload carries no such field, and
 * `--mode` and Shift+Tab both change the live mode without writing it down. So
 * the choice has to be the user's.
 */
function WatchRow({
    watchesFileEdits,
    onChange,
    busy,
}: {
    watchesFileEdits: boolean;
    onChange: (watchFileEdits: boolean) => void;
    busy: boolean;
}) {
    const options: [boolean, string][] = [
        [true, 'Commands and file edits'],
        [false, 'Commands only'],
    ];

    return (
        <div className="flex min-w-0 items-center gap-2.5 border-t border-page-line pt-2.5">
            <span className="h-2 w-2 shrink-0" />
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="font-ui text-xs leading-tight font-semibold text-fg">
                    Raise a toast for
                </div>
                <div className="font-mono text-[10.5px] leading-tight text-fg3">
                    {watchesFileEdits
                        ? 'file edits included — right for default mode'
                        : 'commands only — right for accept-edits mode'}
                </div>
            </div>
            <div
                role="group"
                className="flex shrink-0 overflow-hidden rounded-[7px] border border-border-soft"
            >
                {options.map(([value, label]) => (
                    <button
                        key={label}
                        type="button"
                        disabled={busy}
                        aria-pressed={watchesFileEdits === value}
                        onClick={() => value !== watchesFileEdits && onChange(value)}
                        className={
                            'cursor-pointer px-2.5 py-2 font-small text-[11px] font-semibold ' +
                            'transition-colors disabled:cursor-default disabled:opacity-55 ' +
                            (watchesFileEdits === value
                                ? 'bg-accent text-accent-on'
                                : 'bg-fill text-fg2 hover:bg-fill-hover')
                        }
                    >
                        {label}
                    </button>
                ))}
            </div>
        </div>
    );
}

/**
 * The switch that decides whether an Antigravity toast has an Approve button.
 *
 * Kept apart from Connect: connecting is reversible and harmless, whereas this
 * hands Antigravity a standing instruction to stop asking and trusts AgentToast
 * to ask instead. Worth its own deliberate click.
 */
function ApprovalRow({
    approval,
    onDone,
    onError,
}: {
    approval: AgyApprovalStatus;
    onDone: () => void;
    onError: (message: string) => void;
}) {
    const [busy, setBusy] = useState(false);
    const on = approval.enabled;
    // deny and ask both beat allow in Antigravity, so the user's own rules win
    // and Approve quietly does nothing for anything they cover.
    const shadowed = on && approval.shadowed_by.length > 0;

    const detail = shadowed
        ? `your own rules still win: ${approval.shadowed_by.join(', ')}`
        : on
          ? `on — Antigravity granted ${approval.grants.join(' and ')}`
          : 'off — the toast can deny, but approving means going to the terminal';

    return (
        <div className="flex min-w-0 items-center gap-2.5 border-t border-page-line pt-2.5">
            <Dot tone={shadowed ? 'warn' : on ? 'ok' : 'fg3'} className="h-2 w-2" />
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="font-ui text-xs leading-tight font-semibold text-fg">
                    Approve from the toast
                </div>
                <div
                    className={`path-ellipsis font-mono text-[10.5px] leading-tight ${shadowed ? 'text-warn' : 'text-fg3'}`}
                    title={`${detail}\n${approval.path}`}
                >
                    {detail}
                </div>
            </div>
            <Button
                variant={on ? 'outline' : 'primary'}
                disabled={busy}
                onClick={async () => {
                    setBusy(true);
                    try {
                        await (on ? disableAgyApproval() : enableAgyApproval());
                    } catch (e) {
                        console.error('Could not change Antigravity approval:', e);
                        onError(String(e));
                    }
                    setBusy(false);
                    onDone();
                }}
            >
                {on ? 'Turn off' : 'Turn on'}
            </Button>
        </div>
    );
}

function AntigravityPanel({ active }: { active: boolean }) {
    const { data, error, refresh } = usePolled(agyStatus, POLL_MS, active);
    const [failure, setFailure] = useState<string | null>(null);
    const [rewiring, setRewiring] = useState(false);

    const connected = Boolean(data?.connected && !data.stale);
    // Only read once the hooks are in: the grants are safe only for as long as
    // something is there to answer for them, so there is nothing to show until.
    const approval = usePolled(agyApprovalStatus, POLL_MS, active && connected);

    const note = failure
        ? failure
        : error
          ? 'Could not read Antigravity hooks'
          : !data
            ? undefined
            : connected
              ? // Antigravity reads hooks once, when a session starts. A session
                // already open when Connect ran has loaded none of ours and will
                // keep prompting in the terminal, with nothing anywhere to say
                // why. This line is the only warning there is.
                'Connected — restart any agy session that is already open'
              : 'Not connected — Antigravity will not send anything yet';

    function reload() {
        void refresh();
        void approval.refresh();
    }

    // Changing the scope rewrites hooks.json, which is also what Connect does —
    // so it is the same call, with the new scope.
    async function setWatch(watchFileEdits: boolean) {
        setRewiring(true);
        try {
            await connectAgy(watchFileEdits);
            setFailure(null);
        } catch (e) {
            console.error('Could not change what Antigravity watches:', e);
            setFailure(String(e));
        }
        setRewiring(false);
        reload();
    }

    // The only mode signal there is, and it is a weak one — the startup default,
    // which --mode and Shift+Tab both override without recording it. Weak enough
    // to advise on and never to act on, so it says its piece and leaves the
    // choice alone.
    const modeClash =
        approval.data?.agent_mode === 'accept-edits' && approval.data.watches_file_edits;

    return (
        <div className="flex flex-col gap-2">
            <Panel
                title="Antigravity"
                note={note}
                noteTone={failure || error ? 'warn' : 'fg3'}
            >
                {data ? (
                    <HookRow
                        status={data}
                        globalLabel="This machine"
                        // Antigravity keeps one hooks file for the whole machine,
                        // so there is no scope to choose and nothing per-project
                        // to pass.
                        // A first Connect writes the wider scope; the row
                        // below is where it gets narrowed.
                        onConnect={() => connectAgy(true)}
                        onDisconnect={disconnectAgy}
                        onDone={reload}
                        onError={setFailure}
                    />
                ) : null}
                {connected && approval.data ? (
                    <>
                        <WatchRow
                            watchesFileEdits={approval.data.watches_file_edits}
                            onChange={(v) => void setWatch(v)}
                            busy={rewiring}
                        />
                        <ApprovalRow
                            approval={approval.data}
                            onDone={reload}
                            onError={setFailure}
                        />
                    </>
                ) : null}
            </Panel>
            <Hint>
                Antigravity keeps its hooks in one file for the whole machine, so there is nothing
                to set up per project. Other tools’ hooks in that file are left alone. It reads
                them when a session starts, so a session already open when you connect keeps
                asking in the terminal until you restart it.
            </Hint>
            {modeClash ? (
                <Hint>
                    <span className="text-warn">
                        Antigravity starts in <code>accept-edits</code>, where it does not pause
                        for file edits at all — but AgentToast is still raising a toast for each
                        one. “Commands only” above puts that back the way the mode intends.
                        (Antigravity does not record mode changes made with <code>--mode</code> or
                        Shift+Tab, so this is going on the startup default alone.)
                    </span>
                </Hint>
            ) : null}
            <Hint>
                Commands and file edits are gated differently. Commands go through Antigravity’s
                permission rules in every execution mode, so a toast for one is always right.
                File edits go through the execution mode itself — <code>default</code> pauses for
                a diff review, <code>accept-edits</code> does not pause at all — and nothing in a
                hook payload says which mode is running, so that half is yours to set.
            </Hint>
            <Hint>
                An Antigravity hook cannot approve a call — only block one. So Approve works the
                other way round: AgentToast grants Antigravity the tools it watches, and then
                answers for them. Turn it on and the toast becomes the only thing standing between
                the agent and your machine. If AgentToast is running, that is exactly what you
                want. If it is not running — uninstalled, or its bridge deleted — Antigravity
                treats the missing hook as consent and stops asking. Disconnect gives the grants
                back.
            </Hint>
        </div>
    );
}

export function Connectors({ active }: { active: boolean }) {
    return (
        <div className="flex flex-col gap-4">
            <ClaudePanel active={active} />
            <AntigravityPanel active={active} />
        </div>
    );
}
