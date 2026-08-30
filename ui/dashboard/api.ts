// Everything that talks to the Rust side, plus the polling hook the tabs share.
//
// The dashboard polls rather than subscribing. The registry is small, and a poll
// also picks up whatever changed while this window was closed — which a
// subscription opened on mount would miss.

import { useCallback, useEffect, useRef, useState } from 'react';
import type { AgyApprovalStatus, HookStatus, RemoteStatus, Session } from './types.ts';

// `withGlobalTauri` is on, so the API comes off the window rather than from an
// import. That keeps the front end free of an npm package that has to be kept in
// step with the Rust crate's version.
export const invoke = <T,>(command: string, args?: Record<string, unknown>): Promise<T> =>
    window.__TAURI__.core.invoke<T>(command, args);

export interface Polled<T> {
    data: T | null;
    error: unknown;
    refresh: () => Promise<void>;
}

/**
 * Poll a command on an interval, with a way to re-read it at once.
 *
 * `enabled` is how a hidden tab stops costing anything: the timer is torn down
 * rather than left running behind a tab nobody is looking at.
 */
export function usePolled<T>(
    read: () => Promise<T>,
    intervalMs: number,
    enabled = true,
): Polled<T> {
    const [data, setData] = useState<T | null>(null);
    const [error, setError] = useState<unknown>(null);

    // Held in a ref so that passing a fresh arrow function on every render does
    // not restart the timer, which would reset the interval each time the parent
    // re-rendered and, with a 2s poll, mean it never fired.
    const readRef = useRef(read);
    readRef.current = read;

    const refresh = useCallback(async () => {
        try {
            setData(await readRef.current());
            setError(null);
        } catch (e) {
            console.error('Poll failed:', e);
            setError(e);
        }
    }, []);

    useEffect(() => {
        if (!enabled) return undefined;
        void refresh();
        const timer = setInterval(() => void refresh(), intervalMs);
        return () => clearInterval(timer);
    }, [enabled, intervalMs, refresh]);

    return { data, error, refresh };
}

/* -------------------------------------------------------------- sessions --- */

export const getSessions = () => invoke<Session[]>('get_sessions');

export const respondToEvent = (eventId: string, action: string) =>
    invoke<void>('respond_to_event', { eventId, action, textInput: null });

export const reopenToast = (eventId: string) => invoke<void>('reopen_toast', { eventId });

/* ------------------------------------------------------------ connectors --- */

export const claudeStatus = () => invoke<HookStatus[]>('hook_status', { project: null });
export const connectClaude = (scope: string, project: string | null) =>
    invoke<HookStatus>('connect_hooks', { scope, project });
export const disconnectClaude = (scope: string, project: string | null) =>
    invoke<HookStatus>('disconnect_hooks', { scope, project });

export const addProject = (project: string) => invoke<void>('add_project', { project });
export const removeProject = (project: string) => invoke<void>('remove_project', { project });

export const agyStatus = () => invoke<HookStatus>('agy_hook_status');
export const connectAgy = (watchFileEdits: boolean) =>
    invoke<HookStatus>('connect_agy_hooks', { watchFileEdits });
export const disconnectAgy = () => invoke<HookStatus>('disconnect_agy_hooks');

export const agyApprovalStatus = () => invoke<AgyApprovalStatus>('agy_approval_status');
export const enableAgyApproval = () => invoke<AgyApprovalStatus>('enable_agy_approval');
export const disableAgyApproval = () => invoke<AgyApprovalStatus>('disable_agy_approval');

/* --------------------------------------------------------------- remote --- */

export const remoteStatus = () => invoke<RemoteStatus>('remote_status');
export const setRemoteEnabled = (enabled: boolean) =>
    invoke<RemoteStatus>('set_remote_enabled', { enabled });
export const setRemoteApprove = (allow: boolean) =>
    invoke<RemoteStatus>('set_remote_approve', { allow });
export const setRemotePort = (port: number) => invoke<RemoteStatus>('set_remote_port', { port });
export const startRemotePairing = () => invoke<RemoteStatus>('start_remote_pairing');
export const cancelRemotePairing = () => invoke<RemoteStatus>('cancel_remote_pairing');
export const revokeRemoteDevice = (deviceId: string) =>
    invoke<RemoteStatus>('revoke_remote_device', { deviceId });
export const revokeAllRemoteDevices = () => invoke<RemoteStatus>('revoke_all_remote_devices');

/**
 * Ask for a folder.
 *
 * Resolved at call time, not at module load: the dialog plugin is not always
 * exposed on the global, and reaching for it up front took the whole dashboard
 * down with it.
 */
export async function chooseFolder(): Promise<string | null> {
    const options = { directory: true, title: 'Choose a project folder' };

    const dialog = window.__TAURI__?.dialog;
    if (dialog && typeof dialog.open === 'function') {
        return dialog.open(options);
    }
    // The plugin command works even when its JS wrapper is absent.
    return invoke<string | null>('plugin:dialog|open', { options });
}
