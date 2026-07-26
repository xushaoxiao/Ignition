// Mock, in-browser campaign store (localStorage). Stands in for the config-write API until the
// real tenant-admin endpoints land. Same shape the backend will expose, so swapping to fetch()
// later is a one-file change.
import { newTrackingId } from "./trackingId";
import type { Campaign, CampaignInput } from "./types";

const KEY = "ignition.console.campaigns";

function read(): Campaign[] {
  if (typeof window === "undefined") return [];
  try {
    return JSON.parse(window.localStorage.getItem(KEY) ?? "[]") as Campaign[];
  } catch {
    return [];
  }
}

function write(campaigns: Campaign[]): void {
  window.localStorage.setItem(KEY, JSON.stringify(campaigns));
}

export function listCampaigns(): Campaign[] {
  return read();
}

export function createCampaign(input: CampaignInput, now: string): Campaign {
  const all = read();
  const id = all.reduce((max, c) => Math.max(max, c.id), 0) + 1;
  const campaign: Campaign = {
    ...input,
    id,
    trackingId: newTrackingId(),
    status: "active",
    createdAt: now,
  };
  write([campaign, ...all]);
  return campaign;
}

export function deleteCampaign(id: number): void {
  write(read().filter((c) => c.id !== id));
}
