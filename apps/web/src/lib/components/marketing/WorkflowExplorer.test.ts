import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import WorkflowExplorer from './WorkflowExplorer.svelte';

describe('workflow explorer', () => {
  afterEach(cleanup);

  it('switches the use-case story and keeps every choice accessible', async () => {
    render(WorkflowExplorer);

    expect(
      screen.getByRole('heading', { name: 'See every print workflow click into place.' })
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Build it into your product' })).toHaveAttribute(
      'aria-expanded',
      'true'
    );
    expect(screen.getByRole('img', { name: 'Inside your product printing with Piqae' }))
      .toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Ship every order' }));

    expect(screen.getByRole('button', { name: 'Ship every order' })).toHaveAttribute(
      'aria-expanded',
      'true'
    );
    expect(screen.getByText(/shipping labels and packing slips/)).toBeVisible();
    expect(screen.getByRole('img', { name: 'Fulfilment printing with Piqae' }))
      .toBeInTheDocument();
  });
});
