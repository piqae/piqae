import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { cloudPricingCatalog } from '$lib/server/pricing';
import PricingCards from './PricingCards.svelte';

describe('homepage pricing', () => {
  afterEach(cleanup);

  it('keeps Cloud billing choices and self-hosting distinct', async () => {
    render(PricingCards, {
      homepage: true,
      plans: cloudPricingCatalog.plans
    });

    expect(screen.getByRole('heading', { name: 'Piqae Free' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Piqae Pro' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Self-hosted' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Explore self-hosting' })).toHaveAttribute(
      'href',
      '/open-source'
    );

    await fireEvent.click(screen.getByRole('button', { name: /Annual/ }));

    expect(screen.getByText('$7.50')).toBeInTheDocument();
    expect(screen.getByText(/\$90 billed annually/)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Choose Pro' })).toHaveAttribute(
      'href',
      '/start?plan=pro&interval=annual&source=home-pricing'
    );
  });
});
