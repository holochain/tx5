import { describe, expect, assert, it, beforeAll, afterAll } from 'vitest';
import { RateLimit } from './rate-limit.ts';

describe('RateLimit', () => {
  it('allows steady rate', async () => {
    const rl = new RateLimit(10);
    for (let now = 0; now < 1000; ++now) {
      expect(rl.trackRequest(now, 1)).toEqual(0);
    }
  });

  it('burst', async () => {
    let now = 100;
    const rl = new RateLimit(10);
    for (let i = 0; i < 10; ++i) {
      expect(rl.trackRequest(now, 1)).toEqual(0);
    }
    expect(rl.trackRequest(now, 1)).toEqual(1);
    now += 1;
    expect(rl.trackRequest(now, 1)).toEqual(1);
    now += 2;
    expect(rl.trackRequest(now, 1)).toEqual(0);
  });
});
