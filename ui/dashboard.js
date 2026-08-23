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

/* ---------------------------------------------------------------- setup --- */

let setupTimer = null;

function setupRow(status) {
    const scope = status.project ? 'project' : 'global';
    const label = status.project ? folderName(status.project) : 'Every project';
    const row = document.createElement('div');
    row.className = 'setup-row';
    // Connected and pointing at this build is green; connected to a bridge that
    // no longer exists is a warning, because it silently does nothing.
    row.style.setProperty('--tone', `var(--${status.stale ? 'warn' : status.connected ? 'ok' : 'fg3'})`);

    const dot = document.createElement('div');
    dot.className = 'setup-dot';

    const labels = document.createElement('div');
    labels.className = 'setup-labels';

    const name = document.createElement('div');
    name.className = 'setup-scope';
    name.textContent = label;

    const detail = document.createElement('div');
    detail.className = 'setup-path' + (status.stale ? ' setup-warn' : '');
    detail.textContent = status.stale
        ? `points at a different build: ${status.bridge}`
        : status.connected
            ? status.path
            : `not connected — ${status.path}`;
    detail.title = detail.textContent;

    labels.append(name, detail);

    const action = document.createElement('button');
    action.type = 'button';
    action.className = `btn ${status.connected && !status.stale ? 'btn-outline' : 'btn-primary'}`;
    action.textContent = status.stale ? 'Repair' : status.connected ? 'Disconnect' : 'Connect';
    action.addEventListener('click', async () => {
        action.disabled = true;
        const command = status.connected && !status.stale ? 'disconnect_hooks' : 'connect_hooks';
        try {
            await invoke(command, { scope, project: status.project });
        } catch (e) {
            console.error('Setup failed:', e);
            note(String(e), true);
        }
        refreshSetup();
    });

    row.append(dot, labels, action);

    if (status.project) {
        const remove = document.createElement('button');
        remove.type = 'button';
        remove.className = 'btn btn-ghost setup-remove';
        remove.textContent = '×';
        remove.title = 'Remove this project from the list (its hooks are left alone)';
        remove.addEventListener('click', async () => {
            try {
                await invoke('remove_project', { project: status.project });
            } catch (e) {
                console.error('Could not remove project:', e);
            }
            refreshSetup();
        });
        row.appendChild(remove);
    }

    return row;
}

function note(text, isError) {
    const el = document.getElementById('setup-note');
    el.textContent = text;
    el.classList.toggle('setup-warn', !!isError);
}

async function refreshSetup() {
    let statuses = [];
    try {
        statuses = await invoke('hook_status', { project: null });
    } catch (e) {
        console.error('Failed to read hook status:', e);
        note('Could not read Claude Code settings', true);
        return;
    }

    const rows = document.getElementById('setup-rows');
    rows.innerHTML = '';
    for (const status of statuses) {
        rows.appendChild(setupRow(status));
    }

    const pick = document.createElement('button');
    pick.type = 'button';
    pick.className = 'btn btn-ghost';
    pick.textContent = 'Add a project…';
    pick.addEventListener('click', pickProject);

    const picker = document.createElement('div');
    picker.className = 'setup-row';
    picker.appendChild(pick);
    rows.appendChild(picker);

    const connected = statuses.some((s) => s.connected && !s.stale);
    note(connected ? '' : 'Not connected — Claude Code will not send anything yet');

    // Keep the panel honest while the window stays open.
    if (!setupTimer) setupTimer = setInterval(() => refreshSetup().catch(() => {}), 4000);
}

function folderName(path) {
    if (!path) return 'This project';
    const parts = path.replace(/\\/g, '/').split('/').filter(Boolean);
    return parts[parts.length - 1] || path;
}

// Resolved at call time, not at load: the dialog plugin is not always exposed
// on the global, and reaching for it up front took the whole dashboard down.
async function chooseFolder() {
    const options = { directory: true, title: 'Choose a project folder' };

    const dialog = window.__TAURI__ && window.__TAURI__.dialog;
    if (dialog && typeof dialog.open === 'function') {
        return dialog.open(options);
    }
    // The plugin command works even when its JS wrapper is absent.
    return invoke('plugin:dialog|open', { options });
}

async function pickProject() {
    try {
        const picked = await chooseFolder();
        if (picked) {
            const dir = typeof picked === 'string' ? picked : String(picked);
            // Recorded by the daemon, so the row is still here next time the
            // dashboard is opened — whether or not it gets connected now.
            await invoke('add_project', { project: dir });
            refreshSetup();
        }
    } catch (e) {
        console.error('Folder picker failed:', e);
        note('Could not open the folder picker', true);
    }
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
refreshSetup().catch((e) => {
    console.error('Setup panel failed:', e);
    note('Setup unavailable: ' + e, true);
});
setInterval(refresh, POLL_MS);
