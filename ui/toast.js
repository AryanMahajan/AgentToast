// AgentToast — toast notification frontend.
//
// One window per attention event. The window label carries the event id, so a
// toast always renders its *own* event instead of whichever one happens to be
// first in the pending list.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWebviewWindow } = window.__TAURI__.webviewWindow;

const LABEL_PREFIX = 'toast-';
const LABEL = getCurrentWebviewWindow().label;
const EVENT_ID = LABEL.startsWith(LABEL_PREFIX) ? LABEL.slice(LABEL_PREFIX.length) : null;

let currentEvent = null;
let revealed = false;
let closing = false;
let timeInterval = null;

const el = (id) => document.getElementById(id);

/* -------------------------------------------------------------- mapping --- */

// SessionState -> how the card presents itself. `timer` is the auto-dismiss
// dwell in seconds; 0 means sticky (action required, per the design's dwell
// rules: approval / question / error never auto-dismiss).
const PRESENTATION = {
    WAITING_FOR_PERMISSION: { status: 'Needs approval', tone: 'warn', timer: 0 },
    WAITING_FOR_CONFIRMATION: { status: 'Question', tone: 'accent', timer: 0 },
    WAITING_FOR_INPUT: { status: 'Question', tone: 'accent', timer: 0 },
    ERROR: { status: 'Failed', tone: 'err', timer: 0 },
    WORKING: { status: 'Running', tone: 'accent', timer: 0, indeterminate: true },
    COMPLETED: { status: 'Done', tone: 'ok', timer: 6 },
    IDLE: { status: 'Queued', tone: 'fg3', timer: 6 },
};

const AGENTS = {
    claude: { name: 'Claude Code', initial: 'C' },
    agy: { name: 'Antigravity', initial: 'A' },
};

const BUTTON_CLASS = {
    approve: 'btn-primary',
    confirm: 'btn-primary',
    deny: 'btn-outline',
    reject: 'btn-outline',
    open_session: 'btn-ghost',
};

function agentInfo(agent) {
    return AGENTS[agent] || { name: agent, initial: (agent || '?').charAt(0).toUpperCase() };
}

/* ------------------------------------------------------------- rendering --- */

// The context blob is pretty-printed JSON. The design's log strip is a single
// ellipsised mono line, so pull out the one field that actually matters.
function logLine(event) {
    if (!event.context) return '';
    try {
        const parsed = JSON.parse(event.context);
        const interesting = parsed.command ?? parsed.file_path ?? parsed.path ??
            parsed.pattern ?? parsed.url ?? parsed.description;
        if (typeof interesting === 'string' && interesting.trim()) {
            return interesting.replace(/\s+/g, ' ').trim();
        }
    } catch (_) {
        // Not JSON — fall through and collapse it as plain text.
    }
    return event.context.replace(/\s+/g, ' ').trim();
}

function relativeTime(timestamp) {
    const elapsed = Math.max(0, Math.floor((Date.now() - new Date(timestamp).getTime()) / 1000));
    if (elapsed < 10) return 'now';
    if (elapsed < 60) return `${elapsed}s`;
    if (elapsed < 3600) return `${Math.floor(elapsed / 60)}m`;
    return `${Math.floor(elapsed / 3600)}h`;
}

function renderToast(event) {
    const look = PRESENTATION[event.state] || PRESENTATION.WORKING;
    const agent = agentInfo(event.agent);
    const card = el('card');

    card.style.setProperty('--tone', `var(--${look.tone})`);

    el('avatar').textContent = agent.initial;
    el('agent').textContent = agent.name;
    el('status').textContent = look.status;
    el('task').textContent = event.message;

    // Shown so that, when several terminals are raised at once, the user knows
    // which project they are looking for.
    const cwd = event.cwd || '';
    el('cwd').textContent = cwd;
    el('cwd').title = cwd;
    el('cwd').hidden = !cwd;

    const log = logLine(event);
    el('log').textContent = log;
    el('log').hidden = !log;

    el('indeterminate').hidden = !look.indeterminate;
    el('progress').hidden = true;

    renderFooter(event);
    startClock(event);
    startTimer(look.timer);
}

function renderFooter(event) {
    const footer = el('footer');
    footer.innerHTML = '';

    const actions = event.actions || [];
    if (!actions.length) {
        footer.hidden = true;
        return;
    }
    footer.hidden = false;

    // A send_text action turns the footer into the reply row from the design.
    const textAction = actions.find((a) => a.type === 'send_text');
    if (textAction) {
        const input = document.createElement('input');
        input.type = 'text';
        input.className = 'reply-input';
        input.placeholder = 'Reply to the agent…';

        const send = document.createElement('button');
        send.type = 'button';
        send.className = 'btn btn-primary';
        send.textContent = textAction.label || 'Send';

        const submit = () => respond('send_text', input.value);
        send.addEventListener('click', submit);
        input.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') submit();
        });

        footer.append(input, send);
        setTimeout(() => input.focus(), 60);
        return;
    }

    for (const action of actions) {
        // "Open session" is the quiet tertiary at the far right of the row.
        if (action.type === 'open_session') {
            const spacer = document.createElement('span');
            spacer.className = 'grow';
            footer.appendChild(spacer);
        }
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = `btn ${BUTTON_CLASS[action.type] || 'btn-outline'}`;
        btn.textContent = action.label;
        btn.addEventListener('click', () => respond(action.type, null));
        footer.appendChild(btn);
    }
}

function startClock(event) {
    if (timeInterval) clearInterval(timeInterval);
    const tick = () => {
        el('time').textContent = relativeTime(event.timestamp);
    };
    tick();
    timeInterval = setInterval(tick, 5000);
}

function startTimer(seconds) {
    const timer = el('timer');
    if (!seconds) {
        timer.hidden = true;
        return;
    }
    timer.hidden = false;
    el('card').style.setProperty('--timer-dur', `${seconds}s`);
    // Only non-blocking states carry a timer, so letting it run out
    // decides nothing.
    el('timer-fill').addEventListener('animationend', () => hide(), { once: true });
}

/* ---------------------------------------------------------------- reveal --- */

// Nothing is painted until the card has been measured and the window has been
// revealed at its final size — that ordering is what keeps a half-styled card
// from flashing on screen.
async function reveal() {
    if (revealed) return;
    revealed = true;

    // Font metrics affect the measured height, but never block the reveal on
    // them: WebView2 can leave fonts.ready pending far longer than a toast
    // should sit invisible.
    if (document.fonts && document.fonts.ready) {
        await Promise.race([
            document.fonts.ready.catch(() => {}),
            new Promise((resolve) => setTimeout(resolve, 150)),
        ]);
    }

    // offsetHeight is the layout height, unaffected by the enter transform.
    const height = el('card').offsetHeight;

    try {
        await invoke('toast_ready', { eventId: EVENT_ID, height });
    } catch (e) {
        console.error('Failed to signal ready:', e);
    }

    requestAnimationFrame(() => el('card').classList.add('enter'));
}

/* --------------------------------------------------------------- actions --- */

async function respond(actionType, textInput) {
    if (!currentEvent || closing) return;
    try {
        await invoke('respond_to_event', {
            eventId: currentEvent.event_id,
            action: actionType,
            textInput: textInput || null,
        });
    } catch (error) {
        console.error('Failed to respond:', error);
    }
    closeToast();
}

// Hide the toast without answering it.
//
// Deliberately does NOT cancel the request: the agent stays blocked and the
// event stays pending, so it can still be answered from the dashboard. Closing
// a toast is "not now", not "decide without me".
async function hide() {
    if (closing) return;
    closing = true;
    if (timeInterval) clearInterval(timeInterval);

    el('card').classList.remove('enter');
    el('card').classList.add('leave');
    await new Promise((resolve) => setTimeout(resolve, 120));

    try {
        // Hidden, not closed: the window survives so a reminder or the
        // dashboard can put this exact card straight back on screen.
        await invoke('hide_toast', { eventId: EVENT_ID });
    } catch (e) {
        console.error('Failed to hide toast:', e);
    }

    // Ready to be shown again.
    closing = false;
    el('card').classList.remove('leave');
    el('card').classList.add('enter');
}

async function closeToast() {
    if (closing) return;
    closing = true;
    if (timeInterval) clearInterval(timeInterval);

    el('card').classList.remove('enter');
    el('card').classList.add('leave');
    await new Promise((resolve) => setTimeout(resolve, 120));

    try {
        await invoke('close_window', { eventId: EVENT_ID });
    } catch (e) {
        console.error('Failed to close window:', e);
    }
}

el('close').addEventListener('click', hide);

document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') hide();
});

/* -------------------------------------------------------------- reminder --- */

// Synthesised rather than shipped as an asset: the CSP allows no remote media,
// and a generated tone needs no file bundled into the binary.
function chime() {
    try {
        const Ctx = window.AudioContext || window.webkitAudioContext;
        if (!Ctx) return;
        const ctx = new Ctx();
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.connect(gain);
        gain.connect(ctx.destination);
        osc.frequency.value = 880;
        gain.gain.setValueAtTime(0.0001, ctx.currentTime);
        gain.gain.exponentialRampToValueAtTime(0.06, ctx.currentTime + 0.01);
        gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.25);
        osc.start();
        osc.stop(ctx.currentTime + 0.26);
        setTimeout(() => ctx.close(), 400);
    } catch (_) {
        // Autoplay policy can refuse audio without a prior gesture; the pulse
        // still carries the reminder.
    }
}

listen('toast-reminder', (e) => {
    const card = el('card');
    card.classList.remove('remind');
    // Force a reflow so the animation restarts on a repeat reminder.
    void card.offsetWidth;
    card.classList.add('remind');
    if (e.payload) chime();
});

/* ------------------------------------------------------------------ boot --- */

// The event may be emitted before this window finishes loading, so ask for it
// by id rather than relying on catching the broadcast.
listen('attention-event', (e) => {
    if (!currentEvent && e.payload && e.payload.event_id === EVENT_ID) {
        currentEvent = e.payload;
        renderToast(currentEvent);
        reveal();
    }
});

async function boot() {
    if (!EVENT_ID) {
        console.error('Toast window has no event id in its label:', LABEL);
        return;
    }
    try {
        const event = await invoke('get_event', { eventId: EVENT_ID });
        if (event) {
            currentEvent = event;
            renderToast(currentEvent);
        }
    } catch (e) {
        console.error('Failed to load event:', e);
    }
    // Reveal either way — the fallback in window.rs will show this window
    // shortly regardless, and an empty card beats a stuck invisible one.
    reveal();
}

boot();
