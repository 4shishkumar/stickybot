// The desktop renderer includes board.h for firmware identity. It never drives GPIO.
#pragma once
enum gpio_num_t {
  GPIO_NUM_NC = -1, GPIO_NUM_11 = 11, GPIO_NUM_19 = 19,
  GPIO_NUM_37 = 37, GPIO_NUM_42 = 42, GPIO_NUM_46 = 46,
};
