import { GAME_CODES, gameFor } from "@ignition/games";

export interface GameInfo {
  code: string;
  title: string;
  blurb: string;
  emoji: string;
}

const META: Record<string, { blurb: string; emoji: string }> = {
  lucky_wheel: { blurb: "经典大转盘，指针转动后停在中奖扇区", emoji: "🎡" },
  scratch_card: { blurb: "刮开涂层，揭晓所得奖品", emoji: "🎴" },
  slot_machine: { blurb: "三轴老虎机，滚动后锁定中奖符号", emoji: "🎰" },
  blind_box: { blurb: "摇一摇盲盒，开箱揭晓惊喜", emoji: "🎁" },
  flip_card: { blurb: "翻开卡牌，中间一张揭晓奖品", emoji: "🃏" },
};

/** The game catalog surfaced in the picker — titles come from the shared registry. */
export const GAMES: GameInfo[] = GAME_CODES.map((code) => ({
  code,
  title: gameFor(code).title,
  blurb: META[code]?.blurb ?? "",
  emoji: META[code]?.emoji ?? "🎮",
}));
