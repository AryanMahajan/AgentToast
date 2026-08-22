// AgentToast — Toast notification frontend

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWebviewWindow } = window.__TAURI__.webviewWindow;

let currentEvent = null;
let timerInterval = null;

// Listen for attention events from the Tauri backend
listen('attention-event', (e) => {
    currentEvent = e.payload;
    renderToast(currentEvent);
});

// Render the toast notification
function renderToast(event) {
    const title = document.getElementById('toast-title');
    const message = document.getElementById('toast-message');
    const context = document.getElementById('toast-context');
    const actions = document.getElementById('toast-actions');
    const icon = document.getElementById('toast-icon');

    // Set title based on agent
    const agentName = event.agent === 'claude' ? 'Claude Code' : 
                      event.agent === 'agy' ? 'Antigravity' : event.agent;
    title.textContent = `${agentName} needs your attention`;

    // Set icon based on state
    switch (event.state) {
        case 'WAITING_FOR_PERMISSION':
            icon.textContent = '🔐';
            break;
        case 'WAITING_FOR_CONFIRMATION':
            icon.textContent = '❓';
            break;
        case 'WAITING_FOR_INPUT':
            icon.textContent = '✏️';
            break;
        case 'ERROR':
            icon.textContent = '❌';
            break;
        default:
            icon.textContent = '⚡';
    }

    // Set message
    message.textContent = event.message;

    // Set context (e.g., command details)
    if (event.context) {
        context.textContent = event.context;
        context.classList.add('visible');
    } else {
        context.classList.remove('visible');
    }

    // Render action buttons
    actions.innerHTML = '';
    for (const action of event.actions) {
        const btn = document.createElement('button');
        btn.className = `toast-btn ${getButtonClass(action.type)}`;
        btn.textContent = action.label;
        btn.addEventListener('click', () => handleAction(action.type));
        actions.appendChild(btn);
    }

    // Start the waiting timer
    startTimer();
}

// Get CSS class for a button based on action type
function getButtonClass(actionType) {
    switch (actionType) {
        case 'approve':
        case 'confirm':
            return 'approve';
        case 'deny':
        case 'reject':
            return 'deny';
        default:
            return 'secondary';
    }
}

// Handle a user action (button click)
async function handleAction(actionType) {
    if (!currentEvent) return;

    try {
        if (actionType === 'open_session') {
            // TODO: Focus the terminal window
            // For now, just approve and close
        }

        await invoke('respond_to_event', {
            eventId: currentEvent.event_id,
            action: actionType,
            textInput: null,
        });

        // Close this toast window
        closeToast();
    } catch (error) {
        console.error('Failed to respond:', error);
    }
}

// Dismiss the toast without responding
document.getElementById('toast-close').addEventListener('click', async () => {
    if (currentEvent) {
        try {
            await invoke('dismiss_event', {
                eventId: currentEvent.event_id,
            });
        } catch (error) {
            console.error('Failed to dismiss:', error);
        }
    }
    closeToast();
});

// Close the toast window
async function closeToast() {
    if (timerInterval) clearInterval(timerInterval);
    const appWindow = getCurrentWebviewWindow();
    await appWindow.close();
}

// Start a timer showing how long the agent has been waiting
function startTimer() {
    if (timerInterval) clearInterval(timerInterval);

    const startTime = currentEvent ? new Date(currentEvent.timestamp) : new Date();
    const timerEl = document.createElement('div');
    timerEl.className = 'toast-timer';
    document.getElementById('toast').appendChild(timerEl);

    function updateTimer() {
        const elapsed = Math.floor((Date.now() - startTime.getTime()) / 1000);
        if (elapsed < 60) {
            timerEl.textContent = `Waiting for ${elapsed}s`;
        } else {
            const mins = Math.floor(elapsed / 60);
            const secs = elapsed % 60;
            timerEl.textContent = `Waiting for ${mins}m ${secs}s`;
        }
    }

    updateTimer();
    timerInterval = setInterval(updateTimer, 1000);
}
