import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Status from './Status.svelte';

describe('Status', () => {
  it('renders a human-readable state without relying on colour alone', () => {
    render(Status, { value: 'delivery_uncertain' });
    expect(screen.getByText('Delivery Uncertain')).toBeInTheDocument();
  });
});
