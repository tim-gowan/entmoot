export const handler = (req) => {
  const q = req.q;
  console.log(q);
  db.find(q);
};
