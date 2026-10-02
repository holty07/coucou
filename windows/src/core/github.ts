// What the GitHub poller reports (integrations.rs, poll_github) and what of it is
// waiting on you. Shared by the island card, the pill's mood and the desktop board.

export interface GithubPr {
  key: string;
  repo: string;
  number: number;
  title: string;
  url: string;
  author: string;
  updatedAt: string;
  draft: boolean;
  /** Your own PRs only: "passing" | "failing" | "pending" | "". */
  checks?: string;
  /** Your own PRs only: GitHub's reviewDecision. */
  review?: string;
}

export interface GithubView {
  /** Someone asked you for a review. */
  reviews: GithubPr[];
  /** Yours, and something needs you: CI failing or changes requested. */
  blocked: GithubPr[];
  /** Yours, nothing to do right now. */
  open: GithubPr[];
}

function prs(v: unknown): GithubPr[] {
  return Array.isArray(v) ? (v as GithubPr[]) : [];
}

export const needsYou = (pr: GithubPr) => pr.checks === "failing" || pr.review === "CHANGES_REQUESTED";

export function githubView(data: Record<string, unknown>): GithubView {
  const mine = prs(data.mine);
  return {
    reviews: prs(data.reviews),
    blocked: mine.filter(needsYou),
    open: mine.filter((pr) => !needsYou(pr)),
  };
}

/** How many things are waiting on you. */
export const githubWaiting = (v: GithubView) => v.reviews.length + v.blocked.length;

/** `owner/repo` + 12 → `repo#12`. */
export const shortRef = (pr: GithubPr) => `${pr.repo.split("/").pop()}#${pr.number}`;

/** One line for a chip or a tooltip. */
export function githubSummary(v: GithubView): string {
  const parts: string[] = [];
  if (v.reviews.length) parts.push(`${v.reviews.length} review${v.reviews.length === 1 ? "" : "s"}`);
  const failing = v.blocked.filter((pr) => pr.checks === "failing").length;
  const changes = v.blocked.length - failing;
  if (failing) parts.push(`${failing} CI failing`);
  if (changes) parts.push(`${changes} change${changes === 1 ? "" : "s"} requested`);
  if (parts.length) return parts.join(" · ");
  return v.open.length ? `${v.open.length} open PR${v.open.length === 1 ? "" : "s"}` : "Nothing waiting";
}

/** What a PR row says about itself, and in which colour. */
export function prStatus(pr: GithubPr, isReview: boolean): { text: string; color: string } {
  if (isReview) return { text: pr.author ? `from ${pr.author}` : "Review", color: "#F5A524" };
  if (pr.checks === "failing") return { text: "CI failing", color: "#F4505E" };
  if (pr.review === "CHANGES_REQUESTED") return { text: "Changes", color: "#F4505E" };
  if (pr.review === "APPROVED") return { text: "Approved", color: "#34D399" };
  if (pr.draft) return { text: "Draft", color: "#6B7079" };
  if (pr.checks === "pending") return { text: "CI running", color: "#3B9EFF" };
  return { text: "Open", color: "#6B7079" };
}
