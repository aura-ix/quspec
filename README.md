# quspec

quspec is a lightweight audio spectrogram visualizer.

## Keybindings

- q: Quit the application
- s: Copy the full spectrogram graph screenshot to clipboard
- a: Load the previous audio file in the folder
- d: Load the next audio file in the folder
- h: Pan left / seek backward in time (when zoomed in)
- l: Pan right / seek forward in time (when zoomed in)
- j: Zoom out (double the viewed window duration)
- k: Zoom in (halve the viewed window duration)
- w: Cycle window size (double the FFT size; hold Shift to halve it)
- c: Switch between stereo channels
- f: Toggle fullscreen mode

# TODO
- windows build (and thus likely file opening dialogue)
- macOS build
- font fallback
- multithreading