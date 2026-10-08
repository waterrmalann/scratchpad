// Release builds use the GUI subsystem so Windows does not open a console window next to the
// app. Debug builds keep the console so log output stays visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {}
