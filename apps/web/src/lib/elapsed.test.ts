import { describe, expect, it } from 'vitest';
import { durationLabel, elapsedLabel, relativeLabel } from './elapsed';

const minutes = (count: number) => count * 60_000;

describe('elapsed vocabulary', () => {
  it('buckets a span into minutes, hours, then days', () => {
    expect(elapsedLabel(minutes(9))).toBe('9m');
    expect(elapsedLabel(minutes(59))).toBe('59m');
    expect(elapsedLabel(minutes(120))).toBe('2h');
    expect(elapsedLabel(minutes(60 * 72))).toBe('3d');
  });

  it('refuses to name a span shorter than its smallest bucket', () => {
    expect(elapsedLabel(0)).toBeNull();
    expect(elapsedLabel(20_000)).toBeNull();
    expect(elapsedLabel(-minutes(5))).toBeNull();
    expect(elapsedLabel(Number.NaN)).toBeNull();
  });

  it('keeps the past-instant rendering unchanged', () => {
    expect(relativeLabel(0)).toBe('now');
    expect(relativeLabel(minutes(9))).toBe('9m ago');
    expect(relativeLabel(minutes(120))).toBe('2h ago');
  });

  it('never lets a duration read as an instant', () => {
    expect(durationLabel(20_000)).toBe('under a minute');
    expect(durationLabel(minutes(120))).toBe('2h');
  });
});
