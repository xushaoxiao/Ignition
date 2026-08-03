/**
 * Layout shell and the two helpers every screen needs.
 *
 * Kept in its own file rather than in `App` because both flows (prize draw, daily decision) use
 * them, and a shared import from `App` would make the module graph circular.
 */
import { ApiError } from './api'

/** Single-column phone layout with Telegram's safe areas honoured. */
export function Shell({ children }: { children: React.ReactNode }) {
  return (
    <main className="mx-auto flex min-h-dvh w-full max-w-md flex-col justify-center gap-6 px-5 pb-[calc(1.5rem+env(safe-area-inset-bottom))] pt-[calc(1.5rem+env(safe-area-inset-top))]">
      {children}
    </main>
  )
}

export function Centered({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex flex-1 items-center justify-center text-center text-white/70">
      {children}
    </div>
  )
}

/** Backend error messages are already end-user facing; pass them through. */
export function messageOf(e: unknown): string {
  if (e instanceof ApiError) return e.message
  return '网络异常，请稍后再试'
}
