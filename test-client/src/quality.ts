type Leg = 'pub' | 'sub';

export function measureQuality(leg: Leg, current?: RTCStatsReport, previous?: RTCStatsReport) {
  let bitrateKbps: number | undefined;
  let rttMs: number | undefined;
  let availableKbps: number | undefined;
  let worstSendLossPct: number | undefined;
  let lost = 0;
  let received = 0;
  current?.forEach((stat) => {
    if (stat.type === 'transport') {
      const pair = current.get(stat.selectedCandidatePairId);
      if (Number.isFinite(pair?.currentRoundTripTime)) rttMs = pair.currentRoundTripTime * 1000;
      if (Number.isFinite(pair?.availableOutgoingBitrate)) availableKbps = pair.availableOutgoingBitrate / 1000;
    }
    if (stat.type !== (leg === 'pub' ? 'outbound-rtp' : 'inbound-rtp')) return;
    const last = previous?.get(stat.id);
    if (!last || last.ssrc !== stat.ssrc || stat.timestamp <= last.timestamp) return;
    if (leg === 'pub' && stat.packetsSent > last.packetsSent) {
      const remote = current.get(stat.remoteId);
      // The remote timestamp is when the latest receiver report arrived locally.
      if (remote?.type === 'remote-inbound-rtp' && stat.timestamp - remote.timestamp <= 10_000 &&
          Number.isFinite(remote.fractionLost) && remote.fractionLost >= 0 && remote.fractionLost <= 1) {
        worstSendLossPct = Math.max(worstSendLossPct ?? 0, remote.fractionLost * 100);
      }
    }
    const field = leg === 'pub' ? 'bytesSent' : 'bytesReceived';
    const bytes = stat[field] - last[field];
    if (Number.isFinite(bytes) && bytes >= 0) {
      bitrateKbps = (bitrateKbps ?? 0) + bytes * 8 / (stat.timestamp - last.timestamp);
    }
    if (leg === 'sub') {
      const packets = stat.packetsReceived - last.packetsReceived;
      const missing = stat.packetsLost - last.packetsLost;
      if (Number.isFinite(packets) && packets >= 0 && Number.isFinite(missing)) {
        received += packets;
        // Late packets can reduce cumulative loss between two reports.
        lost += Math.max(0, missing);
      }
    }
  });
  const receiveLossPct = received + lost ? 100 * lost / (received + lost) : undefined;
  return { bitrateKbps, rttMs, availableKbps, lossPct: leg === 'pub' ? worstSendLossPct : receiveLossPct };
}

type Quality = ReturnType<typeof measureQuality>;

export function monitorQuality(
  read: (leg: Leg) => Promise<RTCStatsReport | undefined>,
  update: (pub: Quality, sub: Quality) => void,
) {
  let previousPub: RTCStatsReport | undefined;
  let previousSub: RTCStatsReport | undefined;
  let timer: number | undefined;
  let stopped = false;
  async function poll() {
    const [pub, sub] = await Promise.all([read('pub').catch(() => undefined), read('sub').catch(() => undefined)]);
    if (stopped) return;
    update(measureQuality('pub', pub, previousPub), measureQuality('sub', sub, previousSub));
    previousPub = pub;
    previousSub = sub;
    timer = window.setTimeout(poll, 2000);
  }
  void poll();
  return () => {
    stopped = true;
    window.clearTimeout(timer);
  };
}
