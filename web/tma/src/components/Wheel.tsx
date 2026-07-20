/**
 * 转盘。
 *
 * **动画只是表演。** 结果在服务端就已经定了，前端拿到 `segment_index` 之后
 * 反推该转到哪个角度 —— 而不是先转、转完再问结果。顺序反过来的话，中奖概率
 * 就落在了客户端手里，奖池成本和后续那条可计费转化都跟着不可信。
 *
 * 用 CSS transform + cubic-bezier 而不是游戏引擎：一个转盘不值得往包体里加
 * 几百 KB，而 transform 走 GPU 合成，在低端安卓机上反而更稳。
 */
import { useEffect, useRef, useState } from 'react'

export interface Segment {
  id: number
  label: string
}

interface Props {
  segments: Segment[]
  /** 目标扇区下标。为 null 时静止。 */
  target: number | null
  spinning: boolean
  onSettled: () => void
}

/** 转够几圈再停。少于 3 圈看着像卡顿，多于 6 圈用户会开始等得不耐烦。 */
const FULL_TURNS = 5
const SPIN_MS = 4200

const PALETTE = [
  '#6366f1', '#ec4899', '#f59e0b', '#10b981',
  '#3b82f6', '#a855f7', '#ef4444', '#14b8a6',
]

export function Wheel({ segments, target, spinning, onSettled }: Props) {
  const [rotation, setRotation] = useState(0)
  // 累计圈数只增不减：每次都从当前角度继续往前转，指针不会突然倒回去。
  const turns = useRef(0)

  useEffect(() => {
    if (!spinning || target === null || segments.length === 0) return

    const step = 360 / segments.length
    // 指针固定在正上方（12 点）。要让第 target 个扇区的中心停在指针下，
    // 整个盘面需要反向转过该扇区中心所在的角度。
    const center = target * step + step / 2
    turns.current += FULL_TURNS
    setRotation(turns.current * 360 - center)

    const t = window.setTimeout(onSettled, SPIN_MS)
    return () => window.clearTimeout(t)
  }, [spinning, target, segments.length, onSettled])

  const step = segments.length > 0 ? 360 / segments.length : 360

  return (
    <div className="relative mx-auto aspect-square w-full max-w-[20rem]">
      {/* 指针 */}
      <div className="absolute left-1/2 top-0 z-10 -translate-x-1/2 -translate-y-1">
        <div className="h-0 w-0 border-x-[0.75rem] border-t-[1.25rem] border-x-transparent border-t-amber-300 drop-shadow" />
      </div>

      <div
        className="h-full w-full rounded-full ring-4 ring-white/15"
        style={{
          transform: `rotate(${rotation}deg)`,
          transition: spinning
            ? // 起步快、尾段极慢：最后几度的悬念是这个交互唯一的乐趣来源
              `transform ${SPIN_MS}ms cubic-bezier(0.12, 0.7, 0.05, 1)`
            : 'none',
        }}
      >
        <svg viewBox="-50 -50 100 100" className="h-full w-full">
          {segments.map((seg, i) => (
            <g key={seg.id}>
              <path d={sector(i * step, step)} fill={PALETTE[i % PALETTE.length]} />
              <text
                x={0}
                y={0}
                transform={labelTransform(i * step + step / 2)}
                textAnchor="middle"
                dominantBaseline="middle"
                fill="white"
                fontSize="5.5"
                fontWeight="600"
              >
                {truncate(seg.label)}
              </text>
            </g>
          ))}
          <circle r="7" fill="#fff" opacity="0.9" />
        </svg>
      </div>
    </div>
  )
}

/**
 * 扇区文字的位置与朝向。
 *
 * 文字沿半径方向排布。下半圈的扇区若不额外翻转 180°，文字会是倒着的 ——
 * 奖品名读不出来，用户就不知道自己在抽什么。
 */
function labelTransform(centerDeg: number): string {
  const flip = centerDeg > 90 && centerDeg < 270
  return `rotate(${centerDeg}) translate(0 -30) rotate(${flip ? 180 : 0})`
}

/** 以圆心为顶点、从 12 点方向顺时针起算的一个扇形路径。 */
function sector(startDeg: number, sweepDeg: number): string {
  const r = 50
  const a0 = ((startDeg - 90) * Math.PI) / 180
  const a1 = ((startDeg + sweepDeg - 90) * Math.PI) / 180
  const x0 = r * Math.cos(a0)
  const y0 = r * Math.sin(a0)
  const x1 = r * Math.cos(a1)
  const y1 = r * Math.sin(a1)
  const largeArc = sweepDeg > 180 ? 1 : 0
  return `M 0 0 L ${x0} ${y0} A ${r} ${r} 0 ${largeArc} 1 ${x1} ${y1} Z`
}

/** 扇区内空间有限，过长的奖品名截断，完整名字在结果页展示。 */
function truncate(s: string): string {
  return s.length > 7 ? s.slice(0, 6) + '…' : s
}
