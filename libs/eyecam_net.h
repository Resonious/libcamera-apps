#pragma once

#include <cstdio>
#include <stdint.h>

extern "C" {
  // pub extern "C" fn eyecam_net_init(name: *const c_char) -> *const c_void {
  // pub extern "C" fn eyecam_net_deinit(state: *const c_void) {
  // pub extern "C" fn eyecam_net_wait_for_connection(state: *mut c_void) -> c_int {
  // pub extern "C" fn eyecam_net_write_video(state: *mut c_void, len: usize, data: *const u8) -> c_int {

  const void *eyecam_net_init();
  const void *eyecam_net_deinit(const void *state);
  int eyecam_net_wait_for_connection(const void *state, const char *name);
  int eyecam_net_write_video(const void *state, size_t len, const void *data, uint64_t duration_us);
}

struct EyecamNet {
  EyecamNet() {
    state = eyecam_net_init();
  }
  ~EyecamNet() {
    eyecam_net_deinit(state);
  }
  const void *state;
};
