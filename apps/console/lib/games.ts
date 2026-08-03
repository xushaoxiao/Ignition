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

/** Template code of the daily budget-decision game. */
export const DAILY_BUDGET = "daily_budget";

/** Animation skin the daily game uses for its reward draw — mirrors `REWARD_SKIN` in the TMA. */
export const DAILY_REWARD_SKIN = "blind_box";

/**
 * The game catalog surfaced in the picker.
 *
 * The five skins come from the shared registry. `daily_budget` is appended by hand: it is a
 * scored decision game, not an animation over a draw, so it has no entry in `@ignition/games` —
 * that package's contract is "animate a server-decided outcome", and widening it to fit this game
 * would hand every skin props it does not use.
 */
export const GAMES: GameInfo[] = [
  ...GAME_CODES.map((code) => ({
    code,
    title: gameFor(code).title,
    blurb: META[code]?.blurb ?? "",
    emoji: META[code]?.emoji ?? "🎮",
  })),
  {
    code: DAILY_BUDGET,
    title: "每日理财决策",
    blurb: "每天一个理财场景，选完即评分并科普；连续打卡累积理财分与排行榜，答完进入抽奖",
    emoji: "📊",
  },
];
