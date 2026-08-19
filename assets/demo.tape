# VHS tape for the README demo GIF.
# Regenerate with: vhs assets/demo.tape   (outputs assets/demo.gif)
# Install VHS: https://github.com/charmbracelet/vhs
#
# Run this in an environment where the selected toolchains are installed
# (aiken, node, just, Yaci DevKit) so `just test` actually goes green.
# `cardano-init doctor` tells you what's missing.

Output assets/demo.gif

Set Shell "zsh"
Set FontSize 18
Set Width 1100
Set Height 720
Set Padding 24
#Set BorderRadius 16
#Set Margin 10
#Set MarginFill "#ffffff"
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms
Set PlaybackSpeed 1.0

# --- Scaffold a full stack in one command ---
Type "cardano-init --name my-protocol --on-chain aiken --off-chain meshjs --devnet yaci"
Sleep 600ms
Enter
Sleep 3s

# --- Everything builds and tests out of the box ---
Type "cd my-protocol && just test"
Sleep 600ms
Enter
Sleep 50s

# --- Cleanup ---
Hide
Type "cd .. && rm -rf my-protocol"
Enter
