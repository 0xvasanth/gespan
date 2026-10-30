export type SkillCommon = {
  name: string;
  description: string;
  invocation: string;
};

export type BuiltinSkill = SkillCommon & {
  kind: "builtin";
  scope: "builtin";
  source: "gespan";
};
