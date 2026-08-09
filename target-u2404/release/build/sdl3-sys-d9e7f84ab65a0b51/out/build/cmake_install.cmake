# Install script for directory: /root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL

# Set the install prefix
if(NOT DEFINED CMAKE_INSTALL_PREFIX)
  set(CMAKE_INSTALL_PREFIX "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out")
endif()
string(REGEX REPLACE "/$" "" CMAKE_INSTALL_PREFIX "${CMAKE_INSTALL_PREFIX}")

# Set the install configuration name.
if(NOT DEFINED CMAKE_INSTALL_CONFIG_NAME)
  if(BUILD_TYPE)
    string(REGEX REPLACE "^[^A-Za-z0-9_]+" ""
           CMAKE_INSTALL_CONFIG_NAME "${BUILD_TYPE}")
  else()
    set(CMAKE_INSTALL_CONFIG_NAME "Release")
  endif()
  message(STATUS "Install configuration: \"${CMAKE_INSTALL_CONFIG_NAME}\"")
endif()

# Set the component getting installed.
if(NOT CMAKE_INSTALL_COMPONENT)
  if(COMPONENT)
    message(STATUS "Install component: \"${COMPONENT}\"")
    set(CMAKE_INSTALL_COMPONENT "${COMPONENT}")
  else()
    set(CMAKE_INSTALL_COMPONENT)
  endif()
endif()

# Install shared libraries without execute permission?
if(NOT DEFINED CMAKE_INSTALL_SO_NO_EXE)
  set(CMAKE_INSTALL_SO_NO_EXE "1")
endif()

# Is this installation the result of a crosscompile?
if(NOT DEFINED CMAKE_CROSSCOMPILING)
  set(CMAKE_CROSSCOMPILING "FALSE")
endif()

# Set default install directory permissions.
if(NOT DEFINED CMAKE_OBJDUMP)
  set(CMAKE_OBJDUMP "/usr/bin/objdump")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/pkgconfig" TYPE FILE FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/sdl3.pc")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  foreach(file
      "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/libSDL3.so.0.4.10"
      "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/libSDL3.so.0"
      )
    if(EXISTS "${file}" AND
       NOT IS_SYMLINK "${file}")
      file(RPATH_CHECK
           FILE "${file}"
           RPATH "")
    endif()
  endforeach()
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib" TYPE SHARED_LIBRARY FILES
    "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/libSDL3.so.0.4.10"
    "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/libSDL3.so.0"
    )
  foreach(file
      "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/libSDL3.so.0.4.10"
      "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/libSDL3.so.0"
      )
    if(EXISTS "${file}" AND
       NOT IS_SYMLINK "${file}")
      if(CMAKE_INSTALL_DO_STRIP)
        execute_process(COMMAND "/usr/bin/strip" "${file}")
      endif()
    endif()
  endforeach()
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib" TYPE SHARED_LIBRARY FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/libSDL3.so")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib" TYPE STATIC_LIBRARY FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/libSDL3.a")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib" TYPE STATIC_LIBRARY FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/libSDL3_test.a")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  if(EXISTS "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3headersTargets.cmake")
    file(DIFFERENT _cmake_export_file_changed FILES
         "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3headersTargets.cmake"
         "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3headersTargets.cmake")
    if(_cmake_export_file_changed)
      file(GLOB _cmake_old_config_files "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3headersTargets-*.cmake")
      if(_cmake_old_config_files)
        string(REPLACE ";" ", " _cmake_old_config_files_text "${_cmake_old_config_files}")
        message(STATUS "Old export file \"$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3headersTargets.cmake\" will be replaced.  Removing files [${_cmake_old_config_files_text}].")
        unset(_cmake_old_config_files_text)
        file(REMOVE ${_cmake_old_config_files})
      endif()
      unset(_cmake_old_config_files)
    endif()
    unset(_cmake_export_file_changed)
  endif()
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3" TYPE FILE FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3headersTargets.cmake")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  if(EXISTS "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3sharedTargets.cmake")
    file(DIFFERENT _cmake_export_file_changed FILES
         "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3sharedTargets.cmake"
         "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3sharedTargets.cmake")
    if(_cmake_export_file_changed)
      file(GLOB _cmake_old_config_files "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3sharedTargets-*.cmake")
      if(_cmake_old_config_files)
        string(REPLACE ";" ", " _cmake_old_config_files_text "${_cmake_old_config_files}")
        message(STATUS "Old export file \"$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3sharedTargets.cmake\" will be replaced.  Removing files [${_cmake_old_config_files_text}].")
        unset(_cmake_old_config_files_text)
        file(REMOVE ${_cmake_old_config_files})
      endif()
      unset(_cmake_old_config_files)
    endif()
    unset(_cmake_export_file_changed)
  endif()
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3" TYPE FILE FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3sharedTargets.cmake")
  if(CMAKE_INSTALL_CONFIG_NAME MATCHES "^([Rr][Ee][Ll][Ee][Aa][Ss][Ee])$")
    file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3" TYPE FILE FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3sharedTargets-release.cmake")
  endif()
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  if(EXISTS "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3staticTargets.cmake")
    file(DIFFERENT _cmake_export_file_changed FILES
         "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3staticTargets.cmake"
         "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3staticTargets.cmake")
    if(_cmake_export_file_changed)
      file(GLOB _cmake_old_config_files "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3staticTargets-*.cmake")
      if(_cmake_old_config_files)
        string(REPLACE ";" ", " _cmake_old_config_files_text "${_cmake_old_config_files}")
        message(STATUS "Old export file \"$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3staticTargets.cmake\" will be replaced.  Removing files [${_cmake_old_config_files_text}].")
        unset(_cmake_old_config_files_text)
        file(REMOVE ${_cmake_old_config_files})
      endif()
      unset(_cmake_old_config_files)
    endif()
    unset(_cmake_export_file_changed)
  endif()
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3" TYPE FILE FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3staticTargets.cmake")
  if(CMAKE_INSTALL_CONFIG_NAME MATCHES "^([Rr][Ee][Ll][Ee][Aa][Ss][Ee])$")
    file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3" TYPE FILE FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3staticTargets-release.cmake")
  endif()
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  if(EXISTS "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3testTargets.cmake")
    file(DIFFERENT _cmake_export_file_changed FILES
         "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3testTargets.cmake"
         "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3testTargets.cmake")
    if(_cmake_export_file_changed)
      file(GLOB _cmake_old_config_files "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3testTargets-*.cmake")
      if(_cmake_old_config_files)
        string(REPLACE ";" ", " _cmake_old_config_files_text "${_cmake_old_config_files}")
        message(STATUS "Old export file \"$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3/SDL3testTargets.cmake\" will be replaced.  Removing files [${_cmake_old_config_files_text}].")
        unset(_cmake_old_config_files_text)
        file(REMOVE ${_cmake_old_config_files})
      endif()
      unset(_cmake_old_config_files)
    endif()
    unset(_cmake_export_file_changed)
  endif()
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3" TYPE FILE FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3testTargets.cmake")
  if(CMAKE_INSTALL_CONFIG_NAME MATCHES "^([Rr][Ee][Ll][Ee][Aa][Ss][Ee])$")
    file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3" TYPE FILE FILES "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/CMakeFiles/Export/35815d1d52a6ea1175d74784b559bdb6/SDL3testTargets-release.cmake")
  endif()
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/SDL3" TYPE FILE FILES
    "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/SDL3Config.cmake"
    "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/SDL3ConfigVersion.cmake"
    )
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/include/SDL3" TYPE FILE FILES
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_assert.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_asyncio.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_atomic.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_audio.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_begin_code.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_bits.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_blendmode.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_camera.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_clipboard.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_close_code.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_copying.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_cpuinfo.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_dialog.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_dlopennote.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_egl.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_endian.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_error.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_events.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_filesystem.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_gamepad.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_gpu.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_guid.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_haptic.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_hidapi.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_hints.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_init.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_intrin.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_iostream.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_joystick.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_keyboard.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_keycode.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_loadso.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_locale.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_log.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_main.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_main_impl.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_messagebox.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_metal.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_misc.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_mouse.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_mutex.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_oldnames.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_opengl.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_opengl_glext.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_opengles.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_opengles2.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_opengles2_gl2.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_opengles2_gl2ext.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_opengles2_gl2platform.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_opengles2_khrplatform.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_pen.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_pixels.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_platform.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_platform_defines.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_power.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_process.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_properties.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_rect.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_render.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_scancode.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_sensor.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_stdinc.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_storage.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_surface.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_system.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_thread.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_time.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_timer.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_touch.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_tray.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_version.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_video.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_vulkan.h"
    "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/include-revision/SDL3/SDL_revision.h"
    )
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/include/SDL3" TYPE FILE FILES
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_assert.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_common.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_compare.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_crc32.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_font.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_fuzzer.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_harness.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_log.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_md5.h"
    "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/include/SDL3/SDL_test_memory.h"
    )
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/share/licenses/SDL3" TYPE FILE FILES "/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sdl3-src-3.4.10/SDL/LICENSE.txt")
endif()

if(CMAKE_INSTALL_COMPONENT)
  set(CMAKE_INSTALL_MANIFEST "install_manifest_${CMAKE_INSTALL_COMPONENT}.txt")
else()
  set(CMAKE_INSTALL_MANIFEST "install_manifest.txt")
endif()

string(REPLACE ";" "\n" CMAKE_INSTALL_MANIFEST_CONTENT
       "${CMAKE_INSTALL_MANIFEST_FILES}")
file(WRITE "/src/target-u2404/release/build/sdl3-sys-d9e7f84ab65a0b51/out/build/${CMAKE_INSTALL_MANIFEST}"
     "${CMAKE_INSTALL_MANIFEST_CONTENT}")
