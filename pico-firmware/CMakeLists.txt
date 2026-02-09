# Generated Cmake Pico project file

cmake_minimum_required(VERSION 3.13)

set(CMAKE_C_STANDARD 11)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Initialise pico_sdk from installed location
# (note this can come from environment, CMake cache etc)

# == DO NOT EDIT THE FOLLOWING LINES for the Raspberry Pi Pico VS Code Extension to work ==
if(WIN32)
    set(USERHOME $ENV{USERPROFILE})
else()
    set(USERHOME $ENV{HOME})
endif()
set(sdkVersion 2.1.1)
set(toolchainVersion 14_2_Rel1)
set(picotoolVersion 2.1.1)
set(picoVscode ${USERHOME}/.pico-sdk/cmake/pico-vscode.cmake)
if (EXISTS ${picoVscode})
    include(${picoVscode})
endif()
# ====================================================================================
set(PICO_BOARD pico CACHE STRING "Board type")

# Pull in Raspberry Pi Pico SDK (must be before project)
include(pico_sdk_import.cmake)

project(sniffer C CXX ASM)

# Initialise the Raspberry Pi Pico SDK
pico_sdk_init()

# Add executable. Default name is the project name, version 0.1

add_executable(sniffer sniffer.c uart9_rx.pio)

pico_generate_pio_header(sniffer ${CMAKE_CURRENT_LIST_DIR}/uart9_rx.pio)

pico_set_program_name(sniffer "sniffer")
pico_set_program_version(sniffer "0.1")

# Modify the below lines to enable/disable output over UART/USB
pico_enable_stdio_uart(sniffer 0)
pico_enable_stdio_usb(sniffer 1)

pico_define_boot_stage2(slower_boot2 ${PICO_DEFAULT_BOOT_STAGE2_FILE})
target_compile_definitions(slower_boot2 PRIVATE PICO_FLASH_SPI_CLKDIV=4)

pico_set_boot_stage2(sniffer slower_boot2)

# Add the standard library to the build
target_link_libraries(sniffer
        pico_stdlib
        hardware_pio
        hardware_uart
        hardware_dma
        pico_multicore
        pico_stdio
        pico_stdio_usb)

# target_compile_options(sniffer PRIVATE
#     -O3
#     -funroll-loops
#     -finline-functions
#     -ffast-math
#     -flto
#     -march=armv6-m
#     -mtune=cortex-m0plus
#     -mthumb
#     -fno-stack-protector
#     -fomit-frame-pointer
# )

# Add the standard include files to the build
target_include_directories(sniffer PRIVATE
        ${CMAKE_CURRENT_LIST_DIR}
)

pico_add_extra_outputs(sniffer)

