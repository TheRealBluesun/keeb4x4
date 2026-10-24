MEMORY
{
    FLASH : ORIGIN = 0x08008000, LENGTH = 94K
    /* The last 16 bytes are a reset-surviving mailbox shared with the stub. */
    RAM : ORIGIN = 0x20000000, LENGTH = 32752
}
