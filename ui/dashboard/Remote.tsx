// The Remote tab: let a phone on the same network answer a toast.
//
// Everything here is off until someone turns it on, and the panel's job is to
// be honest about what turning it on means. That is why the address, the paired
// devices and the "not encrypted" note are all on screen rather than tucked
// behind a details link — this is the only part of AgentToast that accepts a
// connection from another machine.

import { useEffect, useState } from 'react';
import {
    cancelRemotePairing,
    remoteStatus,
    revokeAllRemoteDevices,
    revokeRemoteDevice,
    setRemoteApprove,
    setRemoteEnabled,
    setRemotePort,
    startRemotePairing,
    usePolled,
} from './api.ts';
import type { RemoteDevice, RemotePairing, RemoteStatus } from './types.ts';
import { Button, Dot, Hint, Panel } from './ui.tsx';

const POLL_MS = 4000;

/** "3 minutes ago", roughly. Precision past the minute helps nobody here. */
function ago(iso: string): string {
    const seconds = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
    if (seconds < 90) return 'just now';
    if (seconds < 3600) return `${Math.round(seconds / 60)} min ago`;
    if (seconds < 86400) return `${Math.round(seconds / 3600)} h ago`;
    return `${Math.round(seconds / 86400)} d ago`;
}

/** A row that runs one command and reports what went wrong. */
function useAction(onDone: () => void, onError: (message: string) => void) {
    const [busy, setBusy] = useState(false);

    const run = async (work: () => Promise<unknown>) => {
        setBusy(true);
        try {
            await work();
            onError('');
        } catch (e) {
            console.error('Remote command failed:', e);
            onError(String(e));
        }
        setBusy(false);
        onDone();
    };

    return { busy, run };
}

/** The master switch, plus where a phone would find it. */
function ListeningRow({
    status,
    onDone,
    onError,
}: {
    status: RemoteStatus;
    onDone: () => void;
    onError: (message: string) => void;
}) {
    const { busy, run } = useAction(onDone, onError);
    const address = status.addresses[0] ?? null;

    const detail = !status.enabled
        ? 'off — nothing is listening'
        : !status.listening
          ? (status.failure ?? 'on, but the server did not start')
          : (address ?? 'on, but this machine has no network address');

    const tone = !status.enabled ? 'fg3' : status.listening && address ? 'ok' : 'warn';

    return (
        <div className="flex min-w-0 items-center gap-2.5">
            <Dot tone={tone} className="h-2 w-2" />
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="font-ui text-xs leading-tight font-semibold text-fg">
                    Answer from your phone
                </div>
                <div
                    className={`path-ellipsis font-mono text-[10.5px] leading-tight ${tone === 'warn' ? 'text-warn' : 'text-fg3'}`}
                    title={detail}
                >
                    {detail}
                </div>
            </div>
            <Button
                variant={status.enabled ? 'outline' : 'primary'}
                disabled={busy}
                onClick={() => void run(() => setRemoteEnabled(!status.enabled))}
            >
                {status.enabled ? 'Turn off' : 'Turn on'}
            </Button>
        </div>
    );
}

/** Whether a paired phone may approve, or only deny. */
function ApproveRow({
    status,
    onDone,
    onError,
}: {
    status: RemoteStatus;
    onDone: () => void;
    onError: (message: string) => void;
}) {
    const { busy, run } = useAction(onDone, onError);
    const on = status.allow_approve;

    return (
        <div className="flex min-w-0 items-center gap-2.5 border-t border-page-line pt-2.5">
            <Dot tone={on ? 'ok' : 'fg3'} className="h-2 w-2" />
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="font-ui text-xs leading-tight font-semibold text-fg">
                    Approve from a phone
                </div>
                <div className="font-mono text-[10.5px] leading-tight text-fg3">
                    {on
                        ? 'on — a paired device can let a command run'
                        : 'off — a paired device can only deny'}
                </div>
            </div>
            <Button
                variant={on ? 'outline' : 'primary'}
                disabled={busy}
                onClick={() => void run(() => setRemoteApprove(!on))}
            >
                {on ? 'Deny only' : 'Allow approve'}
            </Button>
        </div>
    );
}

/**
 * The port.
 *
 * Only worth showing because "something else already has 8787" is a real and
 * otherwise baffling failure — the panel says the server would not start, and
 * this is the field that fixes it.
 */
function PortRow({
    status,
    onDone,
    onError,
}: {
    status: RemoteStatus;
    onDone: () => void;
    onError: (message: string) => void;
}) {
    const { busy, run } = useAction(onDone, onError);
    const [draft, setDraft] = useState(String(status.port));

    // Follow the saved value unless the field is mid-edit, so a poll landing
    // between keystrokes does not overwrite what is being typed.
    useEffect(() => {
        setDraft(String(status.port));
    }, [status.port]);

    const parsed = Number(draft);
    const valid = Number.isInteger(parsed) && parsed >= 1024 && parsed <= 65535;
    const changed = parsed !== status.port;

    return (
        <div className="flex min-w-0 items-center gap-2.5 border-t border-page-line pt-2.5">
            <span className="h-2 w-2 shrink-0" />
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="font-ui text-xs leading-tight font-semibold text-fg">Port</div>
                <div className="font-mono text-[10.5px] leading-tight text-fg3">
                    change it if something else already has this one
                </div>
            </div>
            <input
                type="number"
                min={1024}
                max={65535}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                className={
                    'w-[76px] shrink-0 rounded-[7px] border bg-fill px-2 py-1.5 font-mono ' +
                    'text-[11.5px] text-fg focus-visible:outline-2 focus-visible:outline-accent ' +
                    (valid || draft === '' ? 'border-border-soft' : 'border-err')
                }
            />
            <Button
                disabled={busy || !valid || !changed}
                onClick={() => void run(() => setRemotePort(parsed))}
            >
                Apply
            </Button>
        </div>
    );
}

/** The QR, while a code is outstanding. */
function PairingCard({
    pairing,
    onDone,
    onError,
}: {
    pairing: RemotePairing;
    onDone: () => void;
    onError: (message: string) => void;
}) {
    const { busy, run } = useAction(onDone, onError);
    const [left, setLeft] = useState(0);

    // A local countdown so the code visibly expires. The server is the actual
    // authority — it stops accepting the code whatever this says — and a poll
    // takes the card down a moment later.
    useEffect(() => {
        const tick = () =>
            setLeft(Math.max(0, Math.round((new Date(pairing.expires_at).getTime() - Date.now()) / 1000)));
        tick();
        const timer = setInterval(tick, 1000);
        return () => clearInterval(timer);
    }, [pairing.expires_at]);

    const minutes = Math.floor(left / 60);
    const seconds = String(left % 60).padStart(2, '0');

    return (
        <div className="flex flex-col items-center gap-2.5 border-t border-page-line pt-3">
            {/* The SVG comes from our own Rust QR renderer, not from anything a
                user or a device supplied, so there is nothing here to inject. */}
            <div
                className="rounded-lg bg-white p-2 [&>svg]:block [&>svg]:h-[184px] [&>svg]:w-[184px]"
                dangerouslySetInnerHTML={{ __html: pairing.qr_svg }}
            />
            <div className="font-ui text-[11.5px] leading-tight font-semibold text-fg">
                Scan this with your phone&rsquo;s camera
            </div>
            <div
                className="path-ellipsis w-full text-center font-mono text-[10.5px] text-fg3"
                title={pairing.url}
            >
                {pairing.url}
            </div>
            <div className="font-mono text-[10.5px] text-fg3">
                {left > 0 ? `expires in ${minutes}:${seconds}` : 'expired'}
            </div>
            <Button variant="ghost" disabled={busy} onClick={() => void run(cancelRemotePairing)}>
                Cancel
            </Button>
        </div>
    );
}

function DeviceRow({
    device,
    onDone,
    onError,
}: {
    device: RemoteDevice;
    onDone: () => void;
    onError: (message: string) => void;
}) {
    const { busy, run } = useAction(onDone, onError);

    return (
        <div className="flex min-w-0 items-center gap-2.5 border-t border-page-line pt-2.5">
            <Dot tone="ok" className="h-2 w-2" />
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="font-ui text-xs leading-tight font-semibold text-fg">
                    {device.name}
                </div>
                <div
                    className="font-mono text-[10.5px] leading-tight text-fg3"
                    title={`paired ${new Date(device.paired_at).toLocaleString()}`}
                >
                    last used {ago(device.last_seen_at)}
                </div>
            </div>
            <Button disabled={busy} onClick={() => void run(() => revokeRemoteDevice(device.id))}>
                Revoke
            </Button>
        </div>
    );
}

export function Remote({ active }: { active: boolean }) {
    const { data, error, refresh } = usePolled(remoteStatus, POLL_MS, active);
    const [failure, setFailure] = useState('');
    const { busy: pairing, run } = useAction(() => void refresh(), setFailure);

    if (error && !data) {
        return (
            <Panel title="Local network" note="could not be read" noteTone="err">
                <div className="font-mono text-[11px] text-fg3">{String(error)}</div>
            </Panel>
        );
    }

    if (!data) {
        return (
            <Panel title="Local network" note="loading…">
                <div className="font-mono text-[11px] text-fg3">Reading the remote settings…</div>
            </Panel>
        );
    }

    const live = data.enabled && data.listening;

    return (
        <div className="flex flex-col gap-3">
            <Panel
                title="Local network"
                note={failure || undefined}
                noteTone={failure ? 'err' : 'fg3'}
            >
                <ListeningRow status={data} onDone={() => void refresh()} onError={setFailure} />

                {data.enabled ? (
                    <>
                        <ApproveRow
                            status={data}
                            onDone={() => void refresh()}
                            onError={setFailure}
                        />
                        <PortRow status={data} onDone={() => void refresh()} onError={setFailure} />
                    </>
                ) : null}

                {live && data.pairing ? (
                    <PairingCard
                        pairing={data.pairing}
                        onDone={() => void refresh()}
                        onError={setFailure}
                    />
                ) : null}

                {live && !data.pairing ? (
                    <div className="flex justify-center border-t border-page-line pt-3">
                        <Button
                            variant="primary"
                            disabled={pairing}
                            onClick={() => void run(startRemotePairing)}
                        >
                            Pair a device
                        </Button>
                    </div>
                ) : null}
            </Panel>

            {data.devices.length > 0 ? (
                <Panel
                    title="Paired devices"
                    note={`${data.devices.length} paired`}
                    footer={
                        <div className="flex justify-end pt-1">
                            <Button
                                variant="ghost"
                                onClick={() => void run(revokeAllRemoteDevices)}
                            >
                                Revoke all
                            </Button>
                        </div>
                    }
                >
                    {/* The first row draws its own top border like the rest, which
                        reads as a divider under the panel title rather than a
                        stray line. */}
                    {data.devices.map((device) => (
                        <DeviceRow
                            key={device.id}
                            device={device}
                            onDone={() => void refresh()}
                            onError={setFailure}
                        />
                    ))}
                </Panel>
            ) : null}

            <Hint>
                A paired phone has to be on the same wifi. There is no relay and no account —
                nothing leaves your network, and no server anywhere knows this machine exists.
                That also means it does nothing over mobile data.
            </Hint>
            <Hint>
                Pairing hands the phone its own key, which is why the QR is good for five minutes
                and only once. Revoke takes that key back immediately. If you lose a phone, revoke
                it here — or turn the whole thing off, which stops the server outright.
            </Hint>
            <Hint>
                <span className="text-warn">
                    Traffic is not encrypted. Anyone who can watch your network can read what a
                    request says and, in principle, act as your phone. That is a reasonable trade
                    on a home or office network and a bad one on public wifi.
                </span>
            </Hint>
            <Hint>
                Alerts only arrive while the page is open — a browser tab cannot wake a phone that
                is asleep in a pocket. The desktop toast is still the thing that always fires.
            </Hint>
        </div>
    );
}
