export class RateLimit {
  gracePeriodMs: number;
  nextAllowedTime: number;

  constructor(gracePeriodMs: number) {
    this.gracePeriodMs = gracePeriodMs;
    this.nextAllowedTime = 0;
  }

  trackRequest(now: number, reqWeightMs: number): number {
    this.nextAllowedTime = Math.max(now, this.nextAllowedTime);
    this.nextAllowedTime += reqWeightMs;

    return Math.max(0, this.nextAllowedTime - now - this.gracePeriodMs);
  }
}
