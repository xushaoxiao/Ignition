export const en = {
  meta: {
    title: "Ignition — Attribution you can put on an invoice",
    description:
      "Ignition powers private-domain gamified growth and stitches every reward back to deterministic, auditable attribution — the kind you can bill on.",
  },
  nav: {
    features: "Capabilities",
    how: "How it works",
    trust: "Trust",
    pricing: "Pricing",
    faq: "FAQ",
    docs: "Docs",
    cta: "Get a demo",
  },
  hero: {
    badge: "An attribution machine — not a gamification toolkit",
    titleLead: "Attribution you can",
    titleEmph: "put on an invoice.",
    subtitle:
      "Ignition runs private-domain gamified growth — spin-to-win, claim codes, referrals — and stitches every reward back to a deterministic, auditable conversion. The wheel earns attention; the attribution path and the append-only ledger earn trust.",
    ctaPrimary: "Get a demo",
    ctaSecondary: "Read the attribution policy",
    note: "Deterministic billing · Double-entry ledger · Per-tenant isolation",
    stats: [
      { value: "100%", label: "deterministic billing basis" },
      { value: "Append-only", label: "double-entry ledger" },
      { value: "Versioned", label: "public attribution policy" },
    ],
  },
  positioning: {
    kicker: "Positioning",
    title: "Not a gamification toolkit — an attribution machine.",
    body: "The spin wheel is acquisition theatre. The attribution path and the ledger are the product, and every engineering priority follows that judgement — because revenue correctness is not a feature you bolt on later.",
    points: [
      "Probabilistic conversions can inform dashboards — they never reach an invoice.",
      "A platform fee, not a cut of inflated numbers, keeps incentives honest.",
      "Every billable event freezes the policy version and evidence that produced it.",
    ],
  },
  features: {
    kicker: "Capabilities",
    title: "Built so the invoice is never a guess",
    subtitle:
      "Six things that make attribution defensible from the first tap all the way to the ledger.",
    items: [
      {
        title: "Deterministic attribution only",
        desc: "Billing depends on confirmable events — claim-code redemption, not probabilistic device matching. If we can't verify it ourselves, it never bills.",
      },
      {
        title: "Append-only double-entry ledger",
        desc: "Refunds and chargebacks post reversing entries; originals are never mutated. An unbalanced transaction is unrepresentable at the type level.",
      },
      {
        title: "Versioned, public policy",
        desc: "Attribution rules are published to customers and KOLs. Every record freezes its deciding policy version and an evidence snapshot — appeal-ready by design.",
      },
      {
        title: "Gamified acquisition",
        desc: "A server-authoritative spin wheel inside a Telegram Mini App: weighted draws, atomic stock, idempotent plays. The client only animates the outcome.",
      },
      {
        title: "Per-tenant isolation",
        desc: "Row-level security scopes every query to its tenant; secrets are encrypted at rest with envelope encryption. Fail-closed by default.",
      },
      {
        title: "Caps that don't stop service",
        desc: "Over-cap conversions still attribute and still credit the KOL — they're simply marked free. Better UX, and a natural upsell.",
      },
    ],
  },
  how: {
    kicker: "How it works",
    title: "One path, end to end",
    subtitle:
      "From a tap in the Mini App to a line on a double-entry ledger — every hop is a fact we can reproduce.",
    steps: [
      {
        step: "01",
        title: "Play",
        desc: "A user opens the Telegram Mini App and spins. The server draws the outcome and issues a single claim code.",
      },
      {
        step: "02",
        title: "Redeem",
        desc: "The code is redeemed inside the customer's app. Telegram identity and app identity bind here — in one locked transaction, the sole stitch point of the whole path.",
      },
      {
        step: "03",
        title: "Attribute",
        desc: "A deterministic attribution and its billable event are written together, stamped with the policy version and an evidence snapshot.",
      },
      {
        step: "04",
        title: "Settle",
        desc: "At month-end, unsettled events are invoiced against the tenant cap and a draft is pushed to the payment gateway.",
      },
      {
        step: "05",
        title: "Ledger & audit",
        desc: "Every charge posts balanced double-entry lines; a daily audit re-checks the invariants and alerts on any drift.",
      },
    ],
  },
  trust: {
    kicker: "Why you can trust the number",
    title: "Four constraints we don't bend",
    subtitle:
      "Most of the system only makes sense against these. They're the reason the invoice holds up.",
    items: [
      {
        code: "C1",
        title: "Billing rides on deterministic attribution",
        desc: "Probabilistic matches may appear on dashboards; they never appear on invoices. One exhaustive check enforces it — forgetting can't silently bill.",
      },
      {
        code: "C2",
        title: "No conflict of interest",
        desc: "A platform fee means we don't earn more by inflating numbers. Rules are versioned and public; every record keeps its evidence.",
      },
      {
        code: "C3",
        title: "The ledger is append-only",
        desc: "Refunds and fraud findings write reverse entries; the basis of an issued invoice stays frozen. The database itself revokes update and delete.",
      },
      {
        code: "C4",
        title: "Capabilities are data-driven",
        desc: "No scattered plan checks. Entitlements come from data, so a promise like “Discord free for now” is recorded, not hard-coded.",
      },
    ],
  },
  pricing: {
    kicker: "Pricing",
    title: "Priced on confirmed conversions",
    subtitle:
      "A platform fee keeps incentives aligned; performance billing only ever counts events we can verify.",
    perMonth: "",
    mostPopular: "Most popular",
    plans: [
      {
        name: "Starter",
        price: "Platform fee",
        tagline: "For a first private-domain campaign",
        features: [
          "Gamified Mini App wheel",
          "Deterministic attribution",
          "Public attribution policy",
          "Monthly cap included",
        ],
        cta: "Start free",
        highlighted: false,
      },
      {
        name: "Growth",
        price: "Fee + performance",
        tagline: "For teams billing KOLs on results",
        features: [
          "Everything in Starter",
          "Performance billing on redemptions",
          "Double-entry ledger & daily audit",
          "Per-tenant isolation & encrypted secrets",
          "Attribution query API",
        ],
        cta: "Get a demo",
        highlighted: true,
      },
      {
        name: "Enterprise",
        price: "Let's talk",
        tagline: "For regulated, multi-brand growth",
        features: [
          "Everything in Growth",
          "Cloud KMS & Stripe adapters",
          "Appeal channel & evidence export",
          "Dedicated onboarding & SLA",
        ],
        cta: "Contact us",
        highlighted: false,
      },
    ],
  },
  faq: {
    kicker: "FAQ",
    title: "Questions, answered",
    items: [
      {
        q: "Do I get charged for probabilistic conversions?",
        a: "No. Billing depends only on deterministic attribution — confirmable events like claim-code redemption. Probabilistic matches can inform dashboards but never reach an invoice.",
      },
      {
        q: "What stops attribution numbers from being inflated?",
        a: "A platform-fee model, not a cut of the numbers, plus versioned public rules and a frozen evidence snapshot on every record. There's no incentive — and no easy path — to inflate.",
      },
      {
        q: "What happens when a conversion needs to be reversed?",
        a: "The ledger is append-only. Refunds, chargebacks and fraud findings post reversing entries; the original basis of an issued invoice is never edited.",
      },
      {
        q: "What happens after we hit the monthly cap?",
        a: "Service continues. Over-cap conversions still attribute and still credit the KOL — they're simply marked free on dashboards.",
      },
      {
        q: "How is one customer's data kept from another's?",
        a: "Row-level security scopes every tenant query, secrets are encrypted at rest, and identity is only ever taken from a verified session — never from a request body or header.",
      },
      {
        q: "Can a KOL appeal a rejected conversion?",
        a: "Yes. Every decision stores the policy version and an append-only evidence snapshot, so a dispute can be recomputed against the exact rules that applied.",
      },
    ],
  },
  cta: {
    title: "Ready to make attribution invoice-grade?",
    subtitle:
      "See the full path — play, redeem, attribute, settle, ledger — running on your own campaign.",
    primary: "Get a demo",
    secondary: "Read the docs",
  },
  footer: {
    tagline: "Private-domain gamified growth with end-to-end attribution.",
    rights: "© 2026 Ignition. All rights reserved.",
    columns: [
      {
        title: "Product",
        links: [
          { label: "Capabilities", href: "#features" },
          { label: "How it works", href: "#how" },
          { label: "Pricing", href: "#pricing" },
        ],
      },
      {
        title: "Resources",
        links: [
          {
            label: "Attribution policy",
            href: "https://github.com/xushaoxiao/Ignition/blob/main/docs/product/attribution-policy-v1.md",
          },
          {
            label: "System design",
            href: "https://github.com/xushaoxiao/Ignition/blob/main/docs/design/system-design.md",
          },
          { label: "FAQ", href: "#faq" },
        ],
      },
      {
        title: "Company",
        links: [
          { label: "Get a demo", href: "#cta" },
          { label: "GitHub", href: "https://github.com/xushaoxiao/Ignition" },
        ],
      },
    ],
  },
};
