vim.api.nvim_create_autocmd("WinNew", {
  pattern = "*",
  callback = function()
    local win = vim.api.nvim_get_current_win()
    vim.api.nvim_win_set_config(win, { border = "" })
  end
})
