select distinct c.pytest_node_id
from {control_schema}.test_group_memberships m
join {control_schema}.test_case_inventory c
  on c.repo_id = m.repo_id
 and c.test_case_id = m.test_case_id
where m.repo_id = %s
  and m.test_group_id = %s
order by c.pytest_node_id
