function outer() {
  const nested = () => {
    return 1;
  };
  return nested();
}
