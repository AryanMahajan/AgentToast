// The shapes the Rust side sends back.
//
// Hand-written rather than generated: there are six of them, they change rarely,
// and a generator would be a build step and a dependency for less than a hundred
// lines. The comments name the Rust type each one mirrors, so a change there has
// somewhere obvious to land.

/** `agenttoast_core::session::AgentType` — `Custom` serialises as an object. */
export type AgentType = 'claude_code' | 'antigravity' | { custom: string };

/** `agenttoast_core::state::SessionState`. */
export type SessionState =
    | 'WORKING'
    | 'WAITING_FOR_INPUT'
    | 'WAITING_FOR_PERMISSION'
    | 'WAITING_FOR_CONFIRMATION'
    | 'ERROR'
    | 'COMPLETED'
    | 'IDLE';

/** `agenttoast_core::event::ActionType`. */
export type ActionType = 'approve' | 'deny' | 'confirm' | 'reject' | 'send_text' | 'open_session';

/** `agenttoast_core::event::Action` — `action_type` is renamed to `type`. */
export interface Action {
    type: ActionType;
    label: string;
}

/** `agenttoast_core::event::AttentionEvent`. */
export interface AttentionEvent {
    event_id: string;
    session_id: string;
    agent: string;
    state: SessionState;
    message: string;
    context?: string;
    actions: Action[];
    timestamp: string;
    tool_name?: string;
}

/** `agenttoast_core::session::Session`. */
export interface Session {
    session_id: string;
    agent_type: AgentType;
    process_id?: number;
    working_directory?: string;
    state: SessionState;
    created_at: string;
    last_activity_at: string;
    attention_request?: AttentionEvent;
    metadata: Record<string, string>;
}

/** `agenttoast::hooks::HookStatus`. */
export interface HookStatus {
    /** The project this row is for, or null for the global scope. */
    project: string | null;
    /** The settings file the row reads and writes. */
    path: string;
    exists: boolean;
    connected: boolean;
    /** The bridge command currently configured, if any. */
    bridge: string | null;
    /** Connected, but pointing at a bridge that is not this build. */
    stale: boolean;
}

/** `agenttoast::commands::AgyApprovalStatus`. */
export interface AgyApprovalStatus {
    /** Whether the configured matcher raises toasts for file edits too. */
    watches_file_edits: boolean;
    /**
     * Antigravity's startup execution mode, if written down. Advisory only —
     * `--mode` and Shift+Tab change the live mode without recording it, so this
     * is good enough to warn with and never good enough to act on.
     */
    agent_mode: string | null;
    enabled: boolean;
    path: string;
    grants: string[];
    /** The user's own deny/ask rules that beat our grants, if any. */
    shadowed_by: string[];
}

/** `agenttoast_remote::store::DeviceInfo` — the token is deliberately absent. */
export interface RemoteDevice {
    id: string;
    name: string;
    paired_at: string;
    last_seen_at: string;
}

/** `agenttoast::remote::PairingView`. */
export interface RemotePairing {
    url: string;
    /** Inline SVG, rendered by the Rust side so the dashboard needs no QR library. */
    qr_svg: string;
    expires_at: string;
}

/** `agenttoast::remote::RemoteStatus`. */
export interface RemoteStatus {
    /** The saved setting. */
    enabled: boolean;
    /** Whether a socket is actually bound. Differs from `enabled` when the port
     *  is taken, which is the case worth showing. */
    listening: boolean;
    port: number;
    allow_approve: boolean;
    /** Addresses a phone could use, best guess first. */
    addresses: string[];
    devices: RemoteDevice[];
    failure: string | null;
    pairing: RemotePairing | null;
}

declare global {
    interface Window {
        __TAURI__: {
            core: { invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T> };
            dialog?: { open?: (options: Record<string, unknown>) => Promise<string | null> };
        };
    }
}
