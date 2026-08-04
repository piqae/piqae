import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { cloudPricingCatalog } from '$lib/server/pricing';
import PricingCards from './PricingCards.svelte';

describe('homepage pricing', () => {
  afterEach(cleanup);

  it('keeps the homepage focused on Cloud billing choices', async () => {
    render(PricingCards, {
      homepage: true,
      plans: cloudPricingCatalog.plans
    });

    expect(screen.getByRole('heading', { name: 'Piqae Free' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Piqae Pro' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Self-hosted' })).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: /Annual/ }));

    expect(screen.getByText('$7.50')).toBeInTheDocument();
    expect(screen.getByText(/\$90 billed annually/)).toBeInTheDocument();
    expect(screen.getByText(/1,200 reported-complete jobs/)).toBeInTheDocument();
    expect(screen.getByText(/300,000 reported-complete jobs/)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Choose Pro' })).toHaveAttribute(
      'href',
      '/start?plan=pro&interval=annual&source=home-pricing'
    );
  });
});
