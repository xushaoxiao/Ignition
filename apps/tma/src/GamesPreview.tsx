/**
 * Dev-only game gallery.
 *
 * The real flow needs a Telegram session + backend to open a game. This harness renders every
 * registered game with mock prizes and a "play" button so all skins can be QA'd in a plain browser
 * (`pnpm dev` then open `/?preview`). It exercises the exact same components and `GameProps`
 * contract as {@link App}; it never ships in the normal flow.
 */
import { useCallback, useState } from 'react'
import { gameFor, GAME_CODES, type Segment } from '@ignition/games'

const MOCK: Segment[] = [
  { id: 1, label: '100 金币' },
  { id: 2, label: '500 金币' },
  { id: 3, label: '限定皮肤' },
  { id: 4, label: '谢谢参与' },
  { id: 5, label: '9折券' },
  { id: 6, label: 'iPhone 大奖' },
  { id: 7, label: '再来一次' },
  { id: 8, label: '50 金币' },
]

function Stage({ code }: { code: string }) {
  const { title, Component } = gameFor(code)
  const [round, setRound] = useState(0)
  const [target, setTarget] = useState<number | null>(null)
  const [spinning, setSpinning] = useState(false)

  const onSettled = useCallback(() => setSpinning(false), [])

  function play() {
    if (spinning) return
    setTarget(Math.floor(Math.random() * MOCK.length))
    setSpinning(true)
    setRound((r) => r + 1) // remount for a clean replay
  }

  return (
    <section className="flex flex-col items-center gap-4 rounded-3xl bg-white/5 p-5 ring-1 ring-white/10">
      <div className="flex w-full items-center justify-between">
        <h2 className="text-base font-bold">{title}</h2>
        <code className="text-xs text-white/40">{code}</code>
      </div>
      <div className="flex min-h-[16rem] w-full items-center justify-center">
        {/* key forces a fresh mount per play so reveal-style games reset cleanly */}
        <Component
          key={round}
          segments={MOCK}
          target={target}
          spinning={spinning}
          onSettled={onSettled}
        />
      </div>
      <button
        onClick={play}
        disabled={spinning}
        className="w-full rounded-2xl bg-amber-400 py-3 text-base font-bold text-slate-900 disabled:bg-white/15 disabled:text-white/40 active:bg-amber-500"
      >
        {spinning ? '进行中…' : '试玩一次'}
      </button>
    </section>
  )
}

export default function GamesPreview() {
  return (
    <main className="mx-auto flex w-full max-w-md flex-col gap-6 px-5 py-8">
      <header className="text-center">
        <h1 className="text-xl font-bold">游戏预览</h1>
        <p className="mt-1 text-sm text-white/50">{GAME_CODES.length} 款游戏 · 服务端出结果，前端只做动画</p>
      </header>
      {GAME_CODES.map((code) => (
        <Stage key={code} code={code} />
      ))}
    </main>
  )
}
