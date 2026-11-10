# VHS tape for the README demo GIF, scene 2: `just test` on the project that
# demo.tape just generated. render-demo.sh plays it back 4x faster, because the
# real run installs packages and boots a local devnet; the typed command says so.
# Render with assets/render-demo.sh, not on its own.

Output assets/.demo-2.mp4

Require just

Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 14
Set Width 1600
Set Height 1000
Set Padding 24
Set Theme "Catppuccin Mocha"
# Typing and pauses below are 4x the usual values, so they look normal at 4x.
Set TypingSpeed 180ms

Hide
Type "PS1='$ ' && cd my-protocol && clear"
Enter
Show

# --- Everything builds and tests out of the box, devnet included ---
Type "just test    # played back at 4x"
Sleep 2s
Enter
# Wait for the prompt to return (a screen match only sees the top of a long scrollback).
Wait+Line@600s /^\$\s*$/
Sleep 20s

Hide
Type "echo $? > ../assets/.demo-status; cd .. && rm -rf my-protocol"
Enter
Sleep 1s
