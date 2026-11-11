# VHS tape for the README demo GIF, scene 1: browse the catalog, then scaffold.
# Scene 2 (`just test`) is in demo-test.tape. Render both with:
#
#   assets/render-demo.sh        (outputs assets/demo.gif)
#
# Needs VHS (https://github.com/charmbracelet/vhs), ffmpeg, a Nerd Font for the
# tool icons, and the toolchains of the selected stack (aiken, node, just,
# yaci-devkit + Docker) so `just test` goes green. `cardano-init doctor` tells
# you what's missing.

Output assets/.demo-1.png

Require cardano-init

Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 18
Set Width 1680
Set Height 1000
Set Padding 24
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms
Set Framerate 30

Hide
Type "PS1='$ ' && export CARDANO_INIT_ICONS=always CARDANO_INIT_NO_UPDATE_CHECK=1 && rm -rf my-protocol && clear"
Enter
Show

# --- Every tool, by role, with the id to pass to each flag ---
Type "cardano-init table"
Sleep 500ms
Enter
Sleep 6s

Hide
Type "clear"
Enter
Show

# --- Scaffold a full stack in one command ---
Type "cardano-init --name my-protocol --on-chain aiken --off-chain meshjs --devnet yaci"
Sleep 500ms
Enter
Wait+Screen /just build/
Sleep 5s
