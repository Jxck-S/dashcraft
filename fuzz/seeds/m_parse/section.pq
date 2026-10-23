section Section1;

shared Rate = 2 meta [IsParameterQuery = true];
[DataSource.Kind = "X"]
shared Q = let f = (x as number) as number => x * Rate in f(21);
Private = {1..3};
