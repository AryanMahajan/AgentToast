// AgentToast — dashboard.
//
// Lists every registered session and lets a pending request be answered here
// as well as from its toast. Polls rather than subscribing: the registry is
// small, and a poll also picks up sessions that changed while this window was
// closed.

const { invoke } = window.__TAURI__.core;

const POLL_MS = 2000;

// Mirrors PRESENTATION in toast.js — the same states, shown as a list row.
const STATES = {
    WAITING_FOR_PERMISSION: { label: 'Needs approval', tone: 'warn' },
    WAITING_FOR_CONFIRMATION: { label: 'Question', tone: 'accent' },
    WAITING_FOR_INPUT: { label: 'Question', tone: 'accent' },
    ERROR: { label: 'Failed', tone: 'err' },
    WORKING: { label: 'Working', tone: 'accent' },
    COMPLETED: { label: 'Done', tone: 'ok' },
    IDLE: { label: 'Idle', tone: 'fg3' },
};

const AGENTS = {
    claude_code: 'Claude Code',
    antigravity: 'Antigravity',
};

const BUTTON_CLASS = {
    approve: 'btn-primary',
    confirm: 'btn-primary',
    deny: 'btn-outline',
    reject: 'btn-outline',
    open_session: 'btn-ghost',
};

const el = (id) => document.getElementById(id);

function agentName(agentType) {
    if (typeof agentType === 'string') return AGENTS[agentType] || agentType;
    // AgentType::Custom serialises as { custom: "name" }.
    if (agentType && typeof agentType === 'object') return Object.values(agentType)[0];
    return 'Agent';
}

function relativeTime(timestamp) {
    const elapsed = Math.max(0, Math.floor((Date.now() - new Date(timestamp).getTime()) / 1000));
    if (elapsed < 10) return 'now';
    if (elapsed < 60) return `${elapsed}s`;
    if (elapsed < 3600) return `${Math.floor(elapsed / 60)}m`;
    return `${Math.floor(elapsed / 3600)}h`;
}

function renderSession(session) {
    const look = STATES[session.state] || STATES.WORKING;

    const row = document.createElement('article');
    row.className = 'session';
    row.style.setProperty('--tone', `var(--${look.tone})`);

    const dot = document.createElement('div');
    dot.className = 'session-dot';

    const body = document.createElement('div');
    body.className = 'session-body';

    const head = document.createElement('div');
    head.className = 'session-head';

    const agent = document.createElement('span');
    agent.className = 'session-agent';
    agent.textContent = agentName(session.agent_type);

    const state = document.createElement('span');
    state.className = 'session-state';
    state.textContent = look.label;

    const time = document.createElement('span');
    time.className = 'session-time';
    time.textContent = relativeTime(session.last_activity_at);

    head.append(agent, state, time);
    body.appendChild(head);

    if (session.working_directory) {
        const cwd = document.createElement('div');
        cwd.className = 'session-cwd';
        // RTL truncation keeps the leaf directory visible on long paths.
        cwd.textContent = session.working_directory;
        cwd.title = session.working_directory;
        body.appendChild(cwd);
    }

    if (session.attention_request) {
        body.appendChild(renderRequest(session.attention_request));
    }

    row.append(dot, body);
    return row;
}

function renderRequest(request) {
    const box = document.createElement('div');
    box.className = 'session-request';

    const message = document.createElement('div');
    message.className = 'session-message';
    message.textContent = request.message;
    box.appendChild(message);

    const actions = document.createElement('div');
    actions.className = 'session-actions';

    for (const action of request.actions || []) {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = `btn ${BUTTON_CLASS[action.type] || 'btn-outline'}`;
        btn.textContent = action.label;
        btn.addEventListener('click', async () => {
            // Disable the whole row's actions so a double-click cannot send two
            // responses for one event.
            actions.querySelectorAll('button').forEach((b) => (b.disabled = true));
            try {
                await invoke('respond_to_event', {
                    eventId: request.event_id,
                    action: action.type,
                    textInput: null,
                });
            } catch (e) {
                console.error('Failed to respond:', e);
                actions.querySelectorAll('button').forEach((b) => (b.disabled = false));
            }
            refresh();
        });
        actions.appendChild(btn);
    }

    // A hidden toast leaves the agent blocked with nothing on screen, so offer
    // an explicit way to pull it back rather than only answering from here.
    const spacer = document.createElement('span');
    spacer.style.flex = '1';
    actions.appendChild(spacer);

    const reopen = document.createElement('button');
    reopen.type = 'button';
    reopen.className = 'btn btn-ghost';
    reopen.textContent = 'Show toast';
    reopen.addEventListener('click', async () => {
        try {
            await invoke('reopen_toast', { eventId: request.event_id });
        } catch (e) {
            console.error('Failed to reopen toast:', e);
        }
    });
    actions.appendChild(reopen);

    box.appendChild(actions);
    return box;
}

async function refresh() {
    let sessions = [];
    try {
        sessions = await invoke('get_sessions');
    } catch (e) {
        console.error('Failed to load sessions:', e);
        return;
    }

    // Sessions needing attention first, then most recently active.
    sessions.sort((a, b) => {
        const aWaiting = a.attention_request ? 0 : 1;
        const bWaiting = b.attention_request ? 0 : 1;
        if (aWaiting !== bWaiting) return aWaiting - bWaiting;
        return new Date(b.last_activity_at) - new Date(a.last_activity_at);
    });

    el('count').textContent = sessions.length;

    const list = el('list');
    list.innerHTML = '';

    if (!sessions.length) {
        const empty = document.createElement('p');
        empty.className = 'dash-empty';
        empty.textContent = 'No active sessions.';
        list.appendChild(empty);
        return;
    }

    for (const session of sessions) {
        list.appendChild(renderSession(session));
    }
}

refresh();
setInterval(refresh, POLL_MS);
