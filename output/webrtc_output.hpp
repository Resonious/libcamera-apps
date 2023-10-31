#pragma once

#include "output.hpp"
#include "../libs/eyecam_net.h"

class WebrtcOutput : public Output
{
public:
	WebrtcOutput(VideoOptions const *options);

protected:
	void outputBuffer(void *mem, size_t size, int64_t timestamp_us, uint32_t flags) override;

private:
	EyecamNet net;
	uint64_t last_timestamp_us;
};
