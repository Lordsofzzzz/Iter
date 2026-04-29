/**
 * Session statistics tracker.
 *
 * Tracks token usage, accumulated cost, and turn count
 * across the entire session.
 */

/** Session statistics interface. */
export interface SessionStats {
  tokens: {
    input:       number;
    output:      number;
    cache_read:  number;
    cache_write: number;
    total:       number;
  };
  cost:  number;
  turns: number;
}

/** Tracks session-level statistics. */
export class Stats {
  private tokens: SessionStats['tokens'] = {
    input:       0,
    output:      0,
    cache_read:  0,
    cache_write: 0,
    total:       0,
  };
  private cost = 0;
  private turns = 0;

  /**
   * Returns a copy of current statistics.
   */
  get(): SessionStats {
    return {
      tokens: { ...this.tokens },
      cost:   this.cost,
      turns:  this.turns,
    };
  }

  /**
   * Adds token counts to the session total.
   */
  addTokens(input: number, output: number, cacheRead: number, cacheWrite: number): void {
    this.tokens.input       += input;
    this.tokens.output      += output;
    this.tokens.cache_read  += cacheRead;
    this.tokens.cache_write += cacheWrite;
    this.tokens.total        = this.tokens.input
                                  + this.tokens.output
                                  + this.tokens.cache_read
                                  + this.tokens.cache_write;
  }

  /**
   * Adds cost to the session total.
   */
  addCost(amount: number): void {
    this.cost += amount;
  }

  /**
   * Increments the turn counter.
   */
  incrementTurns(): void {
    this.turns += 1;
  }

  /**
   * Resets all statistics to zero.
   */
  reset(): void {
    this.tokens.input       = 0;
    this.tokens.output      = 0;
    this.tokens.cache_read  = 0;
    this.tokens.cache_write = 0;
    this.tokens.total       = 0;
    this.cost               = 0;
    this.turns              = 0;
  }
}