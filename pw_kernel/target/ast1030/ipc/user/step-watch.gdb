define step_watch
  stepi
  print/x *(unsigned int*)($r0 + 0x24)
end