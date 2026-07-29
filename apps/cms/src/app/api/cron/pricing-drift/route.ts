import { NextResponse } from 'next/server'
import { runPricingDriftCheck } from '@/hooks/publish'

export async function GET(request: Request) {
  const authorization = request.headers.get('authorization')
  if (!process.env.CRON_SECRET || authorization !== `Bearer ${process.env.CRON_SECRET}`) {
    return NextResponse.json({ error: 'unauthorized' }, { status: 401 })
  }
  const result = await runPricingDriftCheck('hourly_cron')
  return NextResponse.json(result, { status: result.drift ? 409 : 200 })
}

