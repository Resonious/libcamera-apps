#include "webrtc_output.hpp"
#include <cstdlib>

WebrtcOutput::WebrtcOutput(VideoOptions const *options)
	: Output(options), net(), last_timestamp_us(0) {
	if (options->codec != "h264") {
		LOG_ERROR("Webrtc only works with h264 for now. Sorry!");
		return;
	}

	LOG(1, "Waiting for RTC connection (namespace: " << options->webrtc << ")");

	int connected = eyecam_net_wait_for_connection(net.state, options->webrtc.c_str());
	LOG(1, "Connected!? " << connected);

	// Just forcibly quit if connection fails for now.
	if (!connected) {
		LOG_ERROR("WebRTC connection failed. Shutting down.");
		std::exit(51);
	}
}

void WebrtcOutput::outputBuffer(void *mem, size_t size, int64_t timestamp_us, uint32_t flags) {
	int success = eyecam_net_write_video(net.state, size, mem, timestamp_us - last_timestamp_us);
	last_timestamp_us = timestamp_us;

	if (!success) {
		LOG_ERROR("Failed to send samples?");
	}
}
