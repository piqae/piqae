import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import Page from './+page.svelte';

const data = {
  authMode: 'workos',
  returnTo: '/dashboard',
  resetToken: '',
  initialStep: 'password'
};

afterEach(cleanup);

describe('first-party sign in', () => {
  it('renders first-party password and Magic Auth choices', async () => {
    render(Page, { data: data as never, form: null });
    expect(screen.getByRole('heading', { name: 'Sign in to Piqae' })).toBeInTheDocument();
    expect(screen.getByLabelText('Email')).toHaveAttribute('autocomplete', 'email');
    expect(screen.getByLabelText('Password')).toHaveAttribute('autocomplete', 'current-password');
    await fireEvent.click(screen.getByRole('button', { name: /Email me a sign-in code/ }));
    expect(screen.getByRole('button', { name: 'Send code' })).toBeInTheDocument();
  });

  it('uses a generic password failure that does not disclose account existence', () => {
    render(Page, { data: data as never, form: { invalid: true, step: 'password' } as never });
    expect(screen.getByRole('alert')).toHaveTextContent(
      'We couldn’t complete that request. Check the details and try again.'
    );
    expect(document.body.textContent).not.toMatch(/not found|does not exist|unknown email/i);
  });

  it('supports one-time-code autocomplete without rendering pending tokens', () => {
    render(Page, {
      data: data as never,
      form: { step: 'verify', notice: 'Enter the verification code sent to your email.' } as never
    });
    expect(screen.getByLabelText('Six-digit code')).toHaveAttribute('autocomplete', 'one-time-code');
    expect(document.body.textContent).not.toContain('pendingAuthenticationToken');
  });

  it('renders accessible first-party TOTP enrollment without provider challenge tokens', () => {
    render(Page, {
      data: data as never,
      form: {
        step: 'mfa-enroll',
        enrollment: { qrCode: 'data:image/png;base64,test', secret: 'SETUPKEY' }
      } as never
    });
    expect(screen.getByAltText('QR code for adding Piqae to an authenticator app')).toBeInTheDocument();
    expect(screen.getByLabelText('Authenticator code')).toHaveAttribute('autocomplete', 'one-time-code');
    expect(screen.getByText('SETUPKEY')).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('pendingAuthenticationToken');
  });
});
