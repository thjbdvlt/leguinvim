__leguinvim__ - neovim gui with non-monospace font support.

for people who want to use neovim for prose writing: sf novel, master thesis...

may look awful with some plugins. still very buggy.

forked from [neovim-gtk](https://github.com/lyude/neovim-gtk).
(lot of removals and changes.)

```bash
# download linux binary (other platforms binaries will come soons!)
wget https://github.com/thjbdvlt/leguinvim/releases/latest/download/leguinvim
cp leguinvim ~/.local/bin/ # or wherever you want to install it

# or build (requires cargo)
git clone https://github.com/thjbdvlt/leguinvim leguinvim
cd leguinvim
make install
```

the name is a reference to sf writer [ursula k le guin](https://de.wikipedia.org/wiki/Ursula_K._Le_Guin): neovim makes possible to edit text at the speed of thought, just as fast as le guin's characters with their telepathy abilities and their ansibles.
the project could thus have been named *gtkleguinvim*. but it's too long.

![](./screenshots/completion.png)

![](./screenshots/floating-monospace.png)

![](./screenshots/fzf-lua-monospace.png)

to configure __leguinvim__, use a `ginit.vim` file in your neovim config directory:

```vim
" e.g. ~/.config/nvim/ginit.vim

" main font
call rpcnotify(1, 'Gui', 'Font', 'Liberation Sans 12') " default

" monospace font used for floating windows
call rpcnotify(1, 'Gui', 'FontMono', 'Fira Code 12') " default

" use monospace font for floating windows. 0 or 1 (default)
call rpcnotify(1, 'Gui', 'MonoFloat', 1)
```
