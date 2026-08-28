// The dashboard shell: a header, two tabs, and whichever one is showing.

import { useEffect, useRef, useState } from 'react';
import { agyStatus, claudeStatus, getSessions, usePolled } from './api.ts';
import { Connectors } from './Connectors.tsx';
import { Sessions, sortSessions } from './Sessions.tsx';

const SESSION_POLL_MS = 2000;

type TabId = 'sessions' | 'connectors';

function Tab({
    active,
    onSelect,
    label,
    badge,
    attention,
}: {
    active: boolean;
    onSelect: () => void;
    label: string;
    badge?: number;
    attention?: boolean;
}) {
    return (
        <button
            type="button"
            role="tab"
            aria-selected={active}
            onClick={onSelect}
            className={
                'relative -mb-px flex cursor-pointer items-center gap-2 border-b-2 px-1 pb-2.5 ' +
                'font-ui text-[13px] font-semibold transition-colors ' +
                'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent ' +
                (active
                    ? 'border-accent text-fg'
                    : 'border-transparent text-fg3 hover:text-fg2')
            }
        >
            {label}
            {badge ? (
                <span
                    className={
                        'inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full ' +
                        'px-1.5 font-mono text-[10px] font-semibold ' +
                        (attention
                            ? 'bg-warn text-accent-on'
                            : 'border border-border-soft bg-fill text-fg2')
                    }
                >
                    {badge}
                </span>
            ) : null}
        </button>
    );
}

export function App() {
    const [tab, setTab] = useState<TabId>('sessions');

    // Sessions keep polling behind the Connectors tab, because the count and the
    // "needs you" marker on the tab have to stay honest while you are looking at
    // something else. The connector panels do not: they stop when hidden.
    const { data: rawSessions, refresh } = usePolled(getSessions, SESSION_POLL_MS);
    const sessions = rawSessions === null ? null : sortSessions(rawSessions);
    const waiting = sessions?.filter((s) => s.attention_request).length ?? 0;

    // On a fresh install nothing is wired up, and the Sessions tab can only ever
    // say "no active sessions" — which reads like a fault rather than a missing
    // step. Land on Connectors instead, once, and never fight the user's own
    // choice afterwards.
    const decided = useRef(false);
    useEffect(() => {
        if (decided.current) return;
        decided.current = true;
        void (async () => {
            try {
                const [claude, agy] = await Promise.all([claudeStatus(), agyStatus()]);
                const anyConnected =
                    claude.some((s) => s.connected && !s.stale) || (agy.connected && !agy.stale);
                if (!anyConnected) setTab('connectors');
            } catch {
                // Not worth surfacing: the panels report their own failures, and
                // the default tab is a reasonable place to land regardless.
            }
        })();
    }, []);

    return (
        <div className="flex min-h-screen flex-col">
            <header className="sticky top-0 z-10 border-b border-page-line bg-page px-6 pt-[22px]">
                <div className="flex items-end gap-4">
                    <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                        <div className="font-mono text-[10px] leading-none font-semibold tracking-[0.16em] text-fg3 uppercase">
                            agenttoast
                        </div>
                        <h1 className="font-ui text-xl leading-tight font-semibold tracking-tight text-fg">
                            {tab === 'sessions' ? 'Active sessions' : 'Connectors'}
                        </h1>
                    </div>
                </div>

                <nav role="tablist" className="mt-3.5 flex gap-5 border-b border-page-line">
                    <Tab
                        active={tab === 'sessions'}
                        onSelect={() => setTab('sessions')}
                        label="Sessions"
                        // The count is always how many sessions there are; the
                        // colour is whether any of them is waiting on you.
                        // Swapping the number for the waiting count instead
                        // would make the badge mean two different things.
                        badge={sessions?.length ?? 0}
                        attention={waiting > 0}
                    />
                    <Tab
                        active={tab === 'connectors'}
                        onSelect={() => setTab('connectors')}
                        label="Connectors"
                    />
                </nav>
            </header>

            <main className="flex-1 px-6 pt-4 pb-6">
                {/* Both stay mounted so switching tabs does not throw away a
                    panel's state and re-run every command from scratch. */}
                <div hidden={tab !== 'sessions'}>
                    <Sessions sessions={sessions} onAnswered={() => void refresh()} />
                </div>
                <div hidden={tab !== 'connectors'}>
                    <Connectors active={tab === 'connectors'} />
                </div>
            </main>
        </div>
    );
}
