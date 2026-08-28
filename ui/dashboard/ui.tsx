// The handful of primitives both tabs use.
//
// Tailwind utilities are written out at each call site everywhere except here:
// a button and a status dot appear dozens of times and have to look identical,
// so those two get a component rather than a copied class string.

import type { ReactNode } from 'react';

/** Semantic colours, named the way the tokens are. */
export type Tone = 'accent' | 'ok' | 'warn' | 'err' | 'fg3';

const TONE_TEXT: Record<Tone, string> = {
    accent: 'text-accent',
    ok: 'text-ok',
    warn: 'text-warn',
    err: 'text-err',
    fg3: 'text-fg3',
};

const TONE_BG: Record<Tone, string> = {
    accent: 'bg-accent',
    ok: 'bg-ok',
    warn: 'bg-warn',
    err: 'bg-err',
    fg3: 'bg-fg3',
};

export const toneText = (tone: Tone) => TONE_TEXT[tone];

/** The small round status light that opens a session row or a connector row. */
export function Dot({ tone, className = '' }: { tone: Tone; className?: string }) {
    return <span className={`shrink-0 rounded-full ${TONE_BG[tone]} ${className}`} />;
}

type ButtonVariant = 'primary' | 'outline' | 'ghost';

const VARIANT: Record<ButtonVariant, string> = {
    primary:
        'bg-accent text-accent-on px-[15px] py-2 shadow-[0_1px_2px_rgba(0,0,0,0.18)] ' +
        'hover:brightness-110',
    outline:
        'border border-border-soft bg-fill text-fg px-[15px] py-2 hover:bg-fill-hover',
    ghost: 'bg-transparent text-fg3 px-1.5 py-2 text-[11.5px] hover:text-fg',
};

export function Button({
    variant = 'outline',
    className = '',
    ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant }) {
    return (
        <button
            type="button"
            className={
                'shrink-0 cursor-pointer rounded-[7px] font-small text-xs font-semibold ' +
                'whitespace-nowrap transition-colors ' +
                'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent ' +
                'disabled:cursor-default disabled:opacity-55 ' +
                `${VARIANT[variant]} ${className}`
            }
            {...props}
        />
    );
}

/** A titled card. Both connector panels and the empty states sit in one. */
export function Panel({
    title,
    note,
    noteTone = 'fg3',
    children,
    footer,
}: {
    title: string;
    note?: string;
    noteTone?: Tone;
    children: ReactNode;
    footer?: ReactNode;
}) {
    return (
        <section className="flex flex-col gap-2.5 rounded-xl border border-border-soft bg-surface p-3.5">
            <div className="flex items-baseline gap-2.5">
                <h2 className="font-ui text-[13px] font-semibold text-fg">{title}</h2>
                {note ? (
                    <span className={`font-small text-[11px] leading-snug ${toneText(noteTone)}`}>
                        {note}
                    </span>
                ) : null}
            </div>
            <div className="flex flex-col gap-2">{children}</div>
            {footer}
        </section>
    );
}

/** Explanatory prose under a panel. */
export function Hint({ children }: { children: ReactNode }) {
    return (
        <p className="font-ui text-[11px] leading-relaxed text-pretty text-fg3">{children}</p>
    );
}
