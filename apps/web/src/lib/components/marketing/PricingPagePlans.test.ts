import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { cloudPricingCatalog } from '$lib/server/pricing';
import PricingPagePlans from './PricingPagePlans.svelte';

describe('pricing page plans', () => {
  afterEach(cleanup);

  it('presents Free, Pro, and self-hosted choices with accurate annual pricing', async () => {
    render(PricingPagePlans, { plans: cloudPricingCatalog.plans });

    expect(screen.getByRole('heading', { name: 'Piqae Free' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Piqae Pro' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Self-hosted' })).toBeInTheDocument();
    expect(screen.getByText('Best value')).toBeInTheDocument();
    expect(screen.getAllByText('Unlimited', { selector: 'dd' })).toHaveLength(2);

    await fireEvent.click(screen.getByRole('button', { name: /Annual/ }));

    expect(screen.getByText('$7.50')).toBeInTheDocument();
    expect(screen.getByText('$90 billed annually')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Choose Pro' })).toHaveAttribute(
      'href',
      '/start?plan=pro&interval=annual&source=pricing'
    );
  });
});
