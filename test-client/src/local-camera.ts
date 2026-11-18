/** Owns local camera capture, including permission requests that finish after leaving. */
export class LocalCamera {
  private stream: MediaStream | null = null;
  private generation = 0;

  stop() {
    this.generation++;
    this.stream?.getTracks().forEach((track) => track.stop());
    this.stream = null;
  }

  async start(acquire: () => Promise<MediaStream>): Promise<MediaStreamTrack | null> {
    this.stop();
    const generation = this.generation;
    const stream = await acquire();
    if (generation !== this.generation) {
      stream.getTracks().forEach((track) => track.stop());
      return null;
    }
    const track = stream.getVideoTracks()[0];
    if (!track) {
      stream.getTracks().forEach((track) => track.stop());
      throw new Error('Camera returned no video track');
    }
    this.stream = stream;
    return track;
  }
}
