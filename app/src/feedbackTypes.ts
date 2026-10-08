/** Small host contract shared by the bundled SDK bridge and offline mock. */
export type FeedbackWidget = {
  open(): void;
  close(): void;
  destroy(): void;
};
export type FeedbackCallbacks = {
  /** The dialog's own body-level element, to be EXEMPTED from inerting.
   *  Nullable on purpose: if it cannot be found, the host must degrade to
   *  "inert nothing" rather than inert the dialog along with the app. */
  opened(host: HTMLElement | null): void;
  closed(): void;
  /** `durable: false` = memory only, lost on quit. The SDK reports this
   *  truthfully as of Orfis 3222a74; before that a shed queue still called
   *  back as success, which is why the flag exists rather than a bare call. */
  queued(result: { durable: boolean }): void;
  accepted(): void;
};
