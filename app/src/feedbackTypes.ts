/** Small host contract shared by the bundled SDK bridge and offline mock. */
export type FeedbackWidget = {
  open(): void;
  close(): void;
  destroy(): void;
};
export type FeedbackCallbacks = {
  opened(host: HTMLElement): void;
  closed(): void;
  /** `durable: false` = memory only, lost on quit. The SDK reports this
   *  truthfully as of Orfis 3222a74; before that a shed queue still called
   *  back as success, which is why the flag exists rather than a bare call. */
  queued(result: { durable: boolean }): void;
  accepted(): void;
};
