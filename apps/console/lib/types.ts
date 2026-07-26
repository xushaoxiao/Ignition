export interface Prize {
  /** Client-side row id (stable across edits); the backend assigns real ids on publish. */
  id: string;
  label: string;
  weight: number;
  remaining: number;
}

export interface CampaignInput {
  name: string;
  /** Game template code, e.g. `lucky_wheel`. */
  game: string;
  dailyPlayLimit: number;
  startsAt: string | null;
  endsAt: string | null;
  prizes: Prize[];
}

export interface Campaign extends CampaignInput {
  id: number;
  /** Placement identifier that maps a distribution link back to this campaign (start_param). */
  trackingId: string;
  status: "active";
  createdAt: string;
}
