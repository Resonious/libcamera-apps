libcamera_vid = executable('libcamera-vid', files('libcamera_vid.cpp'),
                           include_directories : include_directories('..'),
                           dependencies: [libcamera_dep, boost_dep, libeyecam_net_dep],
                           link_with : libcamera_app,
                           install : true)
