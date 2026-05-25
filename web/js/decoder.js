/**
 * VP8Decoder — WebCodecs VP8 decoder wrapper
 */
export class VP8Decoder {
    constructor(canvas) {
        this.canvas = canvas;
        this.ctx = canvas.getContext('2d');
        this.decoder = null;
        this.configured = false;
        this.seenKeyframe = false;
        this.needsKeyframe = false;
        this.currentWidth = 0;
        this.currentHeight = 0;
    }

    async configure(width, height) {
        if (this.configured && this.currentWidth === width && this.currentHeight === height) {
            return;
        }

        if (typeof VideoDecoder === 'undefined') {
            throw new Error('WebCodecs VideoDecoder is not supported in this browser.');
        }

        const support = await VideoDecoder.isConfigSupported({
            codec: 'vp8',
            codedWidth: width,
            codedHeight: height,
        });
        if (!support.supported) {
            throw new Error('VP8 decoding is not supported by this browser.');
        }

        if (this.decoder) {
            this.decoder.close();
        }

        this.needsKeyframe = false;
        this.seenKeyframe = false;
        this.currentWidth = width;
        this.currentHeight = height;

        this.decoder = new VideoDecoder({
            output: (frame) => {
                try {
                    this.ctx.drawImage(frame, 0, 0, this.canvas.width, this.canvas.height);
                } catch (e) {
                    // ignore draw errors
                }
                frame.close();
            },
            error: (err) => {
                console.warn('VP8Decoder error:', err);
                this.needsKeyframe = true;
            },
        });

        this.decoder.configure({
            codec: 'vp8',
            codedWidth: width,
            codedHeight: height,
        });

        this.configured = true;
    }

    decode(frameType, timestamp, payload) {
        if (!this.configured || !this.decoder) {
            return;
        }

        const isKeyframe = frameType === 0x01;
        if (isKeyframe) {
            this.seenKeyframe = true;
            this.needsKeyframe = false;
        }

        if (!this.seenKeyframe && !isKeyframe) {
            // Cannot decode delta without a prior keyframe
            this.needsKeyframe = true;
            return;
        }

        const chunk = new EncodedVideoChunk({
            type: isKeyframe ? 'key' : 'delta',
            timestamp: timestamp * 1000,
            data: payload,
        });

        try {
            this.decoder.decode(chunk);
        } catch (e) {
            console.warn('decode() threw:', e);
            this.needsKeyframe = true;
        }
    }

    requestKeyframe() {
        const flag = this.needsKeyframe;
        this.needsKeyframe = false;
        return flag;
    }

    destroy() {
        if (this.decoder) {
            try {
                this.decoder.close();
            } catch (e) {
                // ignore
            }
            this.decoder = null;
        }
        this.configured = false;
        this.seenKeyframe = false;
        this.needsKeyframe = false;
        this.currentWidth = 0;
        this.currentHeight = 0;
    }
}
