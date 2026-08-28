// The Sessions tab: every registered session, and a way to answer a pending
// request from here rather than from its toast.

import { useState } from 'react';
import { reopenToast, respondToEvent } from './api.ts';
import type { AgentType, AttentionEvent, Session, SessionState } from './types.ts';
import { Button, Dot, toneText, type Tone } from './ui.tsx';

// Mirrors PRESENTATION in toast.js — the same states, shown as a list row.
const STATES: Record<SessionState, { label: string; tone: Tone }> = {
    WAITING_FOR_PERMISSION: { label: 'Needs approval', tone: 'warn' },
    WAITING_FOR_CONFIRMATION: { label: 'Question', tone: 'accent' },
    WAITING_FOR_INPUT: { label: 'Question', tone: 'accent' },
    ERROR: { label: 'Failed', tone: 'err' },
    WORKING: { label: 'Working', tone: 'accent' },
    COMPLETED: { label: 'Done', tone: 'ok' },
    IDLE: { label: 'Idle', tone: 'fg3' },
};

const AGENTS: Record<string, string> = {
    claude_code: 'Claude Code',
    antigravity: 'Antigravity',
};

const BUTTON_VARIANT: Record<string, 'primary' | 'outline' | 'ghost'> = {
    approve: 'primary',
    confirm: 'primary',
    deny: 'outline',
    reject: 'outline',
    open_session: 'ghost',
};

function agentName(agentType: AgentType): string {
    if (typeof agentType === 'string') return AGENTS[agentType] ?? agentType;
    // AgentType::Custom serialises as { custom: "name" }.
    return Object.values(agentType)[0] ?? 'Agent';
}

function relativeTime(timestamp: string): string {
    const elapsed = Math.max(0, Math.floor((Date.now() - new Date(timestamp).getTime()) / 1000));
    if (elapsed < 10) return 'now';
    if (elapsed < 60) return `${elapsed}s`;
    if (elapsed < 3600) return `${Math.floor(elapsed / 60)}m`;
    return `${Math.floor(elapsed / 3600)}h`;
}

/** Sessions needing attention first, then most recently active. */
export function sortSessions(sessions: Session[]): Session[] {
    return [...sessions].sort((a, b) => {
        const waiting = Number(!a.attention_request) - Number(!b.attention_request);
        if (waiting !== 0) return waiting;
        return (
            new Date(b.last_activity_at).getTime() - new Date(a.last_activity_at).getTime()
        );
    });
}

function Request({ request, onAnswered }: { request: AttentionEvent; onAnswered: () => void }) {
    // One flag for the whole row: a double-click must not send two responses
    // for one event.
    const [busy, setBusy] = useState(false);

    async function answer(action: string) {
        setBusy(true);
        try {
            await respondToEvent(request.event_id, action);
            onAnswered();
        } catch (e) {
            console.error('Failed to respond:', e);
            setBusy(false);
        }
    }

    return (
        <div className="mt-0.5 flex flex-col gap-2 rounded-lg bg-fill p-2.5">
            <div className="font-ui text-[12.5px] leading-snug font-semibold text-pretty text-fg">
                {request.message}
            </div>
            <div className="flex items-center gap-[7px]">
                {request.actions.map((action) => (
                    <Button
                        key={action.type}
                        variant={BUTTON_VARIANT[action.type] ?? 'outline'}
                        disabled={busy}
                        onClick={() => void answer(action.type)}
                    >
                        {action.label}
                    </Button>
                ))}
                {/* A hidden toast leaves the agent blocked with nothing on
                    screen, so there is an explicit way to pull it back rather
                    than only answering from here. */}
                <Button
                    variant="ghost"
                    className="ml-auto"
                    onClick={() => void reopenToast(request.event_id).catch(console.error)}
                >
                    Show toast
                </Button>
            </div>
        </div>
    );
}

function SessionRow({ session, onAnswered }: { session: Session; onAnswered: () => void }) {
    const look = STATES[session.state] ?? STATES.WORKING;

    return (
        <article className="flex gap-3 rounded-xl border border-border-soft bg-surface p-3.5">
            <Dot tone={look.tone} className="mt-[5px] h-[9px] w-[9px]" />
            <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                <div className="flex min-w-0 items-center gap-2">
                    <span className="font-ui text-[13px] font-semibold whitespace-nowrap text-fg">
                        {agentName(session.agent_type)}
                    </span>
                    <span
                        className={`font-small text-[11.5px] font-semibold whitespace-nowrap ${toneText(look.tone)}`}
                    >
                        {look.label}
                    </span>
                    <span className="ml-auto font-small text-[11px] whitespace-nowrap text-fg3">
                        {relativeTime(session.last_activity_at)}
                    </span>
                </div>

                {session.working_directory ? (
                    <div
                        className="path-ellipsis cursor-text font-mono text-[11px] leading-tight text-fg3 select-text"
                        title={session.working_directory}
                    >
                        {session.working_directory}
                    </div>
                ) : null}

                {session.attention_request ? (
                    <Request request={session.attention_request} onAnswered={onAnswered} />
                ) : null}
            </div>
        </article>
    );
}

export function Sessions({
    sessions,
    onAnswered,
}: {
    sessions: Session[] | null;
    onAnswered: () => void;
}) {
    if (sessions === null) {
        return <p className="py-10 text-center font-ui text-[13px] text-fg3">Loading…</p>;
    }

    if (sessions.length === 0) {
        return (
            <div className="flex flex-col items-center gap-1.5 py-16 text-center">
                <p className="font-ui text-[13px] text-fg3">No active sessions.</p>
                <p className="font-ui text-[11.5px] text-fg3/70">
                    Start Claude Code or Antigravity and this fills in on its own.
                </p>
            </div>
        );
    }

    return (
        <div className="flex flex-col gap-2.5">
            {sessions.map((session) => (
                <SessionRow key={session.session_id} session={session} onAnswered={onAnswered} />
            ))}
        </div>
    );
}
